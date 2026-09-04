// SPDX-License-Identifier: AGPL-3.0-only
//! Retention policies for backups, modelled after the pruning rules of
//! Proxmox Backup Server (<https://pve.proxmox.com/wiki/Backup_and_Restore>).
//!
//! A policy is a set of `keep_*` counters. Every counter buckets the existing
//! backups by a time period (hour, day, ISO week, month, year) and keeps the
//! newest backup of each of the `n` most recent buckets. The counters are
//! additive: a backup survives as soon as a single counter keeps it.
use chrono::DateTime;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How many backups a policy keeps per time period.
///
/// A counter that is unset (or zero) does not contribute to the selection. A
/// policy without a single active counter keeps everything, so that an
/// accidentally empty policy never deletes data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"keep_last": 3, "keep_daily": 7, "keep_weekly": 4, "keep_monthly": 6}))]
pub struct BackupRetention {
    /// Keep the last `n` backups, regardless of when they were taken.
    #[serde(default)]
    pub keep_last: Option<u32>,
    /// Keep the newest backup of each of the last `n` hours that have backups.
    #[serde(default)]
    pub keep_hourly: Option<u32>,
    /// Keep the newest backup of each of the last `n` days that have backups.
    #[serde(default)]
    pub keep_daily: Option<u32>,
    /// Keep the newest backup of each of the last `n` ISO weeks that have backups.
    #[serde(default)]
    pub keep_weekly: Option<u32>,
    /// Keep the newest backup of each of the last `n` months that have backups.
    #[serde(default)]
    pub keep_monthly: Option<u32>,
    /// Keep the newest backup of each of the last `n` years that have backups.
    #[serde(default)]
    pub keep_yearly: Option<u32>,
}

/// A backup a policy is applied to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionCandidate {
    /// The backup id, which is its creation time in milliseconds.
    pub id: i64,
    /// Whether the backup is protected and therefore never removed.
    pub protected: bool,
}

/// The result of applying a [`BackupRetention`], newest backup first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionOutcome {
    /// Backup ids the policy keeps.
    pub keep: Vec<i64>,
    /// Backup ids the policy removes.
    pub remove: Vec<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetentionMark {
    Protected,
    Keep,
    Remove,
}

impl RetentionMark {
    const fn keeps(self) -> bool {
        matches!(self, Self::Protected | Self::Keep)
    }
}

impl BackupRetention {
    /// Whether the policy has no active counter and therefore keeps everything.
    #[must_use]
    pub const fn is_noop(&self) -> bool {
        matches!(
            (
                self.keep_last,
                self.keep_hourly,
                self.keep_daily,
                self.keep_weekly,
                self.keep_monthly,
                self.keep_yearly,
            ),
            (
                None | Some(0),
                None | Some(0),
                None | Some(0),
                None | Some(0),
                None | Some(0),
                None | Some(0),
            )
        )
    }

    /// Splits `candidates` into the backups this policy keeps and the ones it
    /// removes. `timezone` decides which local hour, day, week, month and year
    /// a backup falls into, so that a daily policy keeps the backup of a
    /// calendar day as the operator sees it.
    #[must_use]
    pub fn apply(&self, candidates: &[RetentionCandidate], timezone: Tz) -> RetentionOutcome {
        let mut sorted: Vec<RetentionCandidate> = candidates.to_vec();
        sorted.sort_unstable_by(|first, second| second.id.cmp(&first.id));

        if self.is_noop() {
            return RetentionOutcome {
                keep: sorted.iter().map(|candidate| candidate.id).collect(),
                remove: Vec::new(),
            };
        }

        // Protected backups are never removed. A backup whose id is not a
        // valid timestamp cannot be sorted into any bucket, so no counter could
        // ever justify deleting it either.
        let mut marks: HashMap<i64, RetentionMark> = sorted
            .iter()
            .filter(|candidate| {
                candidate.protected || DateTime::from_timestamp_millis(candidate.id).is_none()
            })
            .map(|candidate| (candidate.id, RetentionMark::Protected))
            .collect();

        for (keep, period) in [
            (self.keep_last, Period::Last),
            (self.keep_hourly, Period::Hourly),
            (self.keep_daily, Period::Daily),
            (self.keep_weekly, Period::Weekly),
            (self.keep_monthly, Period::Monthly),
            (self.keep_yearly, Period::Yearly),
        ] {
            mark_period(&mut marks, &sorted, timezone, keep.unwrap_or_default(), period);
        }

        let mut outcome = RetentionOutcome::default();
        for candidate in sorted {
            if marks
                .get(&candidate.id)
                .copied()
                .is_some_and(RetentionMark::keeps)
            {
                outcome.keep.push(candidate.id);
            } else {
                outcome.remove.push(candidate.id);
            }
        }
        outcome
    }
}

#[derive(Clone, Copy)]
enum Period {
    Last,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Period {
    /// The bucket a backup falls into. Two backups sharing a bucket compete for
    /// the same slot, so only the newer one of them is kept.
    fn bucket(self, candidate: RetentionCandidate, timezone: Tz) -> Option<String> {
        if let Self::Last = self {
            return Some(candidate.id.to_string());
        }
        let time = DateTime::from_timestamp_millis(candidate.id)?.with_timezone(&timezone);
        let format = match self {
            // Handled above, but the compiler cannot know that.
            Self::Last | Self::Hourly => "%Y/%m/%d/%H",
            Self::Daily => "%Y/%m/%d",
            // ISO year and week, so that a week is not split across new year.
            Self::Weekly => "%G/%V",
            Self::Monthly => "%Y/%m",
            Self::Yearly => "%Y",
        };
        Some(time.format(format).to_string())
    }
}

/// Applies a single counter, leaving backups an earlier counter already decided
/// on untouched: a policy is the union of its counters, never the intersection.
fn mark_period(
    marks: &mut HashMap<i64, RetentionMark>,
    sorted: &[RetentionCandidate],
    timezone: Tz,
    keep: u32,
    period: Period,
) {
    let keep = keep as usize;
    if keep == 0 {
        return;
    }

    // Buckets an already kept (or protected) backup covers do not get a second
    // backup kept for them, and do not use up this counter's budget.
    let covered: HashSet<String> = sorted
        .iter()
        .filter(|candidate| {
            marks
                .get(&candidate.id)
                .copied()
                .is_some_and(RetentionMark::keeps)
        })
        .filter_map(|candidate| period.bucket(*candidate, timezone))
        .collect();

    let mut included: HashSet<String> = HashSet::new();
    for candidate in sorted {
        let Some(bucket) = period.bucket(*candidate, timezone) else {
            continue;
        };
        if covered.contains(&bucket) || marks.contains_key(&candidate.id) {
            continue;
        }
        if included.contains(&bucket) {
            // An older backup of a bucket already represented by a newer one.
            marks.insert(candidate.id, RetentionMark::Remove);
            continue;
        }
        if included.len() >= keep {
            // `sorted` is newest first, so everything left is older still.
            break;
        }
        included.insert(bucket);
        marks.insert(candidate.id, RetentionMark::Keep);
    }
}

#[cfg(test)]
mod tests {
    use super::{BackupRetention, RetentionCandidate};
    use chrono::DateTime;
    use chrono_tz::Tz;

    fn at(time: &str) -> i64 {
        DateTime::parse_from_rfc3339(time)
            .expect("test timestamp should be valid")
            .timestamp_millis()
    }

    fn candidates(times: &[&str]) -> Vec<RetentionCandidate> {
        times
            .iter()
            .map(|time| RetentionCandidate {
                id: at(time),
                protected: false,
            })
            .collect()
    }

    #[test]
    fn empty_policy_keeps_everything() {
        let list = candidates(&["2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"]);
        let outcome = BackupRetention::default().apply(&list, Tz::UTC);
        assert_eq!(outcome.keep.len(), 2, "an empty policy must not delete");
        assert!(outcome.remove.is_empty(), "an empty policy must not delete");
    }

    #[test]
    fn keep_last_keeps_the_newest() {
        let list = candidates(&[
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
            "2026-01-03T00:00:00Z",
        ]);
        let outcome = BackupRetention {
            keep_last: Some(2),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert_eq!(
            outcome.keep,
            vec![at("2026-01-03T00:00:00Z"), at("2026-01-02T00:00:00Z")]
        );
        assert_eq!(outcome.remove, vec![at("2026-01-01T00:00:00Z")]);
    }

    #[test]
    fn keep_daily_keeps_the_newest_of_each_day() {
        let list = candidates(&[
            "2026-01-01T01:00:00Z",
            "2026-01-01T23:00:00Z",
            "2026-01-02T01:00:00Z",
            "2026-01-02T23:00:00Z",
            "2026-01-03T12:00:00Z",
        ]);
        let outcome = BackupRetention {
            keep_daily: Some(2),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert_eq!(
            outcome.keep,
            vec![at("2026-01-03T12:00:00Z"), at("2026-01-02T23:00:00Z")]
        );
        assert_eq!(outcome.remove.len(), 3);
    }

    #[test]
    fn counters_are_additive() {
        let list = candidates(&[
            "2026-01-01T12:00:00Z",
            "2026-01-02T12:00:00Z",
            "2026-01-03T10:00:00Z",
            "2026-01-03T12:00:00Z",
        ]);
        // keep_last covers both backups of the 3rd, keep_daily then still keeps
        // the 2nd and the 1st because their days are not covered yet.
        let outcome = BackupRetention {
            keep_last: Some(2),
            keep_daily: Some(2),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert_eq!(outcome.keep.len(), 4);
        assert!(outcome.remove.is_empty());
    }

    #[test]
    fn buckets_follow_the_configured_timezone() {
        // 23:00 UTC on the 1st is 00:00 on the 2nd in Berlin (UTC+1), so under
        // Berlin these two backups fall into different days and both survive.
        let list = candidates(&["2026-01-01T12:00:00Z", "2026-01-01T23:00:00Z"]);
        let policy = BackupRetention {
            keep_daily: Some(2),
            ..BackupRetention::default()
        };
        assert_eq!(policy.apply(&list, Tz::UTC).keep.len(), 1);
        assert_eq!(policy.apply(&list, Tz::Europe__Berlin).keep.len(), 2);
    }

    #[test]
    fn protected_backups_survive_and_cover_their_bucket() {
        let mut list = candidates(&[
            "2026-01-01T01:00:00Z",
            "2026-01-01T23:00:00Z",
            "2026-01-02T12:00:00Z",
        ]);
        let protected = list
            .first_mut()
            .expect("the list should not be empty here, it was just built");
        protected.protected = true;
        let protected = protected.id;

        let outcome = BackupRetention {
            keep_daily: Some(2),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert!(outcome.keep.contains(&protected), "protected must be kept");
        // The 1st is already covered by the protected backup, so the 23:00
        // backup of that day is not kept a second time.
        assert_eq!(outcome.remove, vec![at("2026-01-01T23:00:00Z")]);
    }

    #[test]
    fn weekly_uses_iso_weeks_across_the_year_boundary() {
        // 2025-12-29 and 2026-01-01 are both in ISO week 2026-W01.
        let list = candidates(&["2025-12-29T12:00:00Z", "2026-01-01T12:00:00Z"]);
        let outcome = BackupRetention {
            keep_weekly: Some(2),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert_eq!(outcome.keep, vec![at("2026-01-01T12:00:00Z")]);
        assert_eq!(outcome.remove, vec![at("2025-12-29T12:00:00Z")]);
    }

    #[test]
    fn undatable_ids_are_never_removed() {
        // Out of range for a millisecond timestamp: it belongs to no bucket, so
        // no counter can justify deleting it.
        let list = vec![
            RetentionCandidate {
                id: i64::MAX,
                protected: false,
            },
            RetentionCandidate {
                id: at("2026-01-01T12:00:00Z"),
                protected: false,
            },
        ];
        let outcome = BackupRetention {
            keep_last: Some(1),
            ..BackupRetention::default()
        }
        .apply(&list, Tz::UTC);
        assert!(outcome.keep.contains(&i64::MAX));
        assert!(outcome.remove.is_empty());
    }
}
