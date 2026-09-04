// SPDX-License-Identifier: AGPL-3.0-only
//! Retention policies for backups, modelled after the pruning rules of
//! Proxmox Backup Server (<https://pve.proxmox.com/wiki/Backup_and_Restore>).
//!
//! A policy is a set of `keep_*` counters. Every counter buckets the existing
//! backups by a time period (hour, day, ISO week, month, year) and keeps the
//! newest backup of each of the `n` most recent periods that hold backups.
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// How many backups a policy keeps per time period.
///
/// Each counter makes its selection on its own and the results are unioned, so
/// a backup survives as soon as a single counter keeps it and the order the
/// counters are applied in does not change the outcome. A counter of `0` keeps
/// nothing by itself; a policy whose counters are all `0` keeps everything, so
/// that an empty policy never deletes data.
///
/// Periods are always counted in UTC. In local time a daylight saving change
/// drops or duplicates an hourly period once a year.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({
    "keep_last": 3,
    "keep_hourly": 0,
    "keep_daily": 7,
    "keep_weekly": 4,
    "keep_monthly": 6,
    "keep_yearly": 1
}))]
pub struct BackupRetention {
    /// Keep the last `n` backups, regardless of when they were taken.
    pub keep_last: u32,
    /// Keep the newest backup of each of the last `n` hours that have backups.
    pub keep_hourly: u32,
    /// Keep the newest backup of each of the last `n` days that have backups.
    pub keep_daily: u32,
    /// Keep the newest backup of each of the last `n` ISO weeks that have backups.
    pub keep_weekly: u32,
    /// Keep the newest backup of each of the last `n` months that have backups.
    pub keep_monthly: u32,
    /// Keep the newest backup of each of the last `n` years that have backups.
    pub keep_yearly: u32,
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
    /// Exempt from the policy: kept, and invisible to every counter.
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
        self.keep_last == 0
            && self.keep_hourly == 0
            && self.keep_daily == 0
            && self.keep_weekly == 0
            && self.keep_monthly == 0
            && self.keep_yearly == 0
    }

    /// Splits `candidates` into the backups this policy keeps and the ones it
    /// removes.
    #[must_use]
    pub fn apply(&self, candidates: &[RetentionCandidate]) -> RetentionOutcome {
        // Keyed by id, which is a timestamp, so the map is ordered oldest to
        // newest and reverse iteration walks the backups newest first.
        let mut marks: BTreeMap<i64, RetentionMark> = candidates
            .iter()
            .map(|candidate| {
                // A backup whose id is not a valid timestamp belongs to no
                // period, so no counter could ever justify deleting it.
                let exempt =
                    candidate.protected || DateTime::from_timestamp_millis(candidate.id).is_none();
                let mark = if exempt {
                    RetentionMark::Protected
                } else {
                    RetentionMark::Remove
                };
                (candidate.id, mark)
            })
            .collect();

        if self.is_noop() {
            return RetentionOutcome {
                keep: marks.into_keys().rev().collect(),
                remove: Vec::new(),
            };
        }

        for (keep, period) in [
            (self.keep_last, Period::Last),
            (self.keep_hourly, Period::Hourly),
            (self.keep_daily, Period::Daily),
            (self.keep_weekly, Period::Weekly),
            (self.keep_monthly, Period::Monthly),
            (self.keep_yearly, Period::Yearly),
        ] {
            mark_period(&mut marks, keep, period);
        }

        let mut outcome = RetentionOutcome::default();
        for (id, mark) in marks.into_iter().rev() {
            if mark.keeps() {
                outcome.keep.push(id);
            } else {
                outcome.remove.push(id);
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
    /// The period a backup falls into. Backups sharing a period compete for the
    /// same slot, so only the newest of them is kept.
    fn of(self, id: i64) -> Option<String> {
        if matches!(self, Self::Last) {
            // Every backup is its own period, which makes the counter a plain
            // "keep the newest n".
            return Some(id.to_string());
        }
        let time = DateTime::from_timestamp_millis(id)?;
        let format = match self {
            Self::Last => unreachable!("answered above, before the id is dated"),
            Self::Hourly => "%Y/%m/%d/%H",
            Self::Daily => "%Y/%m/%d",
            // ISO year and week, so that a week is not split at new year.
            Self::Weekly => "%G/%V",
            Self::Monthly => "%Y/%m",
            Self::Yearly => "%Y",
        };
        Some(time.format(format).to_string())
    }
}

/// Keeps the newest backup of each of the `keep` most recent periods.
///
/// Marks are only ever raised to [`RetentionMark::Keep`], never lowered: the
/// counters of a policy are a union, so no counter can take a backup away from
/// another one. What a counter selects does not depend on the marks either,
/// which is what makes the counters order independent.
fn mark_period(marks: &mut BTreeMap<i64, RetentionMark>, keep: u32, period: Period) {
    if keep == 0 {
        return;
    }
    let mut periods: HashSet<String> = HashSet::new();
    // Newest first, so the backup representing a period is the newest one in it.
    for (id, mark) in marks.iter_mut().rev() {
        // Protected backups are exempt from the policy entirely: they neither
        // represent a period nor use up a counter.
        if matches!(*mark, RetentionMark::Protected) {
            continue;
        }
        let Some(current) = period.of(*id) else {
            continue;
        };
        if !periods.insert(current) {
            // An older backup of a period a newer one already represents.
            continue;
        }
        *mark = RetentionMark::Keep;
        if periods.len() >= keep as usize {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackupRetention, RetentionCandidate};
    use chrono::DateTime;

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
        let outcome = BackupRetention::default().apply(&list);
        assert_eq!(
            outcome.keep,
            vec![at("2026-01-02T00:00:00Z"), at("2026-01-01T00:00:00Z")],
            "an empty policy must keep everything, newest first"
        );
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
            keep_last: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
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
            keep_daily: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert_eq!(
            outcome.keep,
            vec![at("2026-01-03T12:00:00Z"), at("2026-01-02T23:00:00Z")]
        );
        assert_eq!(outcome.remove.len(), 3);
    }

    #[test]
    fn keep_monthly_keeps_the_newest_of_each_month() {
        let list = candidates(&[
            "2025-11-30T12:00:00Z",
            "2025-12-01T12:00:00Z",
            "2025-12-31T12:00:00Z",
        ]);
        let outcome = BackupRetention {
            keep_monthly: 1,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert_eq!(outcome.keep, vec![at("2025-12-31T12:00:00Z")]);
        assert_eq!(outcome.remove.len(), 2);
    }

    #[test]
    fn counters_are_a_union() {
        let list = candidates(&[
            "2026-01-01T12:00:00Z",
            "2026-01-02T12:00:00Z",
            "2026-01-03T10:00:00Z",
            "2026-01-03T12:00:00Z",
        ]);
        // keep_last holds both backups of the 3rd, which keep_daily would not,
        // while keep_daily reaches back to the 2nd, which keep_last would not.
        let outcome = BackupRetention {
            keep_last: 2,
            keep_daily: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert_eq!(
            outcome.keep,
            vec![
                at("2026-01-03T12:00:00Z"),
                at("2026-01-03T10:00:00Z"),
                at("2026-01-02T12:00:00Z"),
            ]
        );
        assert_eq!(outcome.remove, vec![at("2026-01-01T12:00:00Z")]);
    }

    #[test]
    fn periods_are_counted_in_utc() {
        // 23:00 UTC and 12:00 UTC are the same UTC day, so they share a daily
        // period even though a timezone east of UTC would split them.
        let list = candidates(&["2026-01-01T12:00:00Z", "2026-01-01T23:00:00Z"]);
        let outcome = BackupRetention {
            keep_daily: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert_eq!(outcome.keep, vec![at("2026-01-01T23:00:00Z")]);
        assert_eq!(outcome.remove, vec![at("2026-01-01T12:00:00Z")]);
    }

    #[test]
    fn protected_backups_are_exempt_from_the_policy() {
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
            keep_daily: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
        // The protected backup is kept without occupying the slot of its day,
        // so the policy still keeps two days of its own on top of it.
        assert_eq!(
            outcome.keep,
            vec![
                at("2026-01-02T12:00:00Z"),
                at("2026-01-01T23:00:00Z"),
                protected,
            ]
        );
        assert!(outcome.remove.is_empty());
    }

    #[test]
    fn weekly_uses_iso_weeks_across_the_year_boundary() {
        // 2025-12-29 and 2026-01-01 are both in ISO week 2026-W01.
        let list = candidates(&["2025-12-29T12:00:00Z", "2026-01-01T12:00:00Z"]);
        let outcome = BackupRetention {
            keep_weekly: 2,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert_eq!(outcome.keep, vec![at("2026-01-01T12:00:00Z")]);
        assert_eq!(outcome.remove, vec![at("2025-12-29T12:00:00Z")]);
    }

    #[test]
    fn undatable_ids_are_never_removed() {
        // Out of range for a millisecond timestamp: it belongs to no period, so
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
            keep_last: 1,
            ..BackupRetention::default()
        }
        .apply(&list);
        assert!(outcome.keep.contains(&i64::MAX));
        assert!(outcome.remove.is_empty());
    }
}
