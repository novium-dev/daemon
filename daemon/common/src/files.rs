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
