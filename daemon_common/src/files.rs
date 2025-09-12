/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use bitflags::bitflags;

bitflags! {
    pub struct FileMode: u32 {
        const FIFO = 0x1000;
        const CHR = 0x2000;
        const DIR = 0x4000;
        const NAM = 0x5000;
        const BLK = 0x6000;
        const REG = 0x8000;
        const LNK = 0xA000;
        const SOCK = 0xC000;
    }
}
