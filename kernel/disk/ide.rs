use alloc::boxed::Box;

use collections::string::String;
use collections::vec::Vec;

use core::ptr;

use arch::memory::Memory;

use disk::Disk;

use drivers::pci::config::PciConfig;
use drivers::io::{Io, Pio, ReadOnly, WriteOnly};

use system::error::{Error, Result, EIO};

/// An disk extent
#[derive(Copy, Clone)]
#[repr(packed)]
pub struct Extent {
    pub block: u64,
    pub length: u64,
}

impl Extent {
    pub fn empty(&self) -> bool {
        return self.block == 0 || self.length == 0;
    }
}

/// Direction of DMA, set if moving from disk to memory, not set if moving from memory to disk
const CMD_DIR: u8 = 1 << 3;
/// DMA should process PRDT
const CMD_ACT: u8 = 1;
/// DMA interrupt occured
const STS_INT: u8 = 1 << 2;
/// DMA error occured
const STS_ERR: u8 = 1 << 1;
/// DMA is processing PRDT
const STS_ACT: u8 = 1;

/// PRDT End of Table
const PRD_EOT: u8 = 1 << 7;

/// Physical Region Descriptor
#[repr(packed)]
struct Prd {
    addr: u32,
    size: u16,
    rsv: u8,
    eot: u8,
}

struct Prdt {
    reg: Pio<u32>,
    mem: Memory<Prd>,
}

impl Prdt {
    fn new(port: u16) -> Self {
        let mut reg = Pio::<u32>::new(port);
        reg.write(0);

        Prdt {
            reg: reg,
            mem: Memory::new_align(512, 65536).unwrap(),
        }
    }
}

impl Drop for Prdt {
    fn drop(&mut self) {
        self.reg.write(0);
    }
}

// Status port bits
const ATA_SR_BSY: u8 = 0x80;
const ATA_SR_DRDY: u8 = 0x40;
const ATA_SR_DF: u8 = 0x20;
const ATA_SR_DSC: u8 = 0x10;
const ATA_SR_DRQ: u8 = 0x08;
const ATA_SR_CORR: u8 = 0x04;
const ATA_SR_IDX: u8 = 0x02;
const ATA_SR_ERR: u8 = 0x01;

// Error port bits
const ATA_ER_BBK: u8 = 0x80;
const ATA_ER_UNC: u8 = 0x40;
const ATA_ER_MC: u8 = 0x20;
const ATA_ER_IDNF: u8 = 0x10;
const ATA_ER_MCR: u8 = 0x08;
const ATA_ER_ABRT: u8 = 0x04;
const ATA_ER_TK0NF: u8 = 0x02;
const ATA_ER_AMNF: u8 = 0x01;

// Commands
const ATA_CMD_READ_PIO: u8 = 0x20;
const ATA_CMD_READ_PIO_EXT: u8 = 0x24;
const ATA_CMD_READ_DMA: u8 = 0xC8;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_PIO: u8 = 0x30;
const ATA_CMD_WRITE_PIO_EXT: u8 = 0x34;
const ATA_CMD_WRITE_DMA: u8 = 0xCA;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_CACHE_FLUSH: u8 = 0xE7;
const ATA_CMD_CACHE_FLUSH_EXT: u8 = 0xEA;
const ATA_CMD_PACKET: u8 = 0xA0;
const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

// Identification
const ATA_IDENT_DEVICETYPE: u8 = 0;
const ATA_IDENT_CYLINDERS: u8 = 2;
const ATA_IDENT_HEADS: u8 = 6;
const ATA_IDENT_SECTORS: u8 = 12;
const ATA_IDENT_SERIAL: u8 = 20;
const ATA_IDENT_MODEL: u8 = 54;
const ATA_IDENT_CAPABILITIES: u8 = 98;
const ATA_IDENT_FIELDVALID: u8 = 106;
const ATA_IDENT_MAX_LBA: u8 = 120;
const ATA_IDENT_COMMANDSETS: u8 = 164;
const ATA_IDENT_MAX_LBA_EXT: u8 = 200;

// Selection
const ATA_MASTER: u8 = 0x00;
const ATA_SLAVE: u8 = 0x01;

// Types
const IDE_ATA: u8 = 0x00;
const IDE_ATAPI: u8 = 0x01;

pub struct Ide;

impl Ide {
    pub fn disks(mut pci: PciConfig) -> Vec<Box<Disk>> {
        let mut ret: Vec<Box<Disk>> = Vec::new();

        unsafe { pci.flag(4, 4, true) }; // Bus mastering

        let busmaster = unsafe { pci.read(0x20) } as u16 & 0xFFF0;

        debug!("Primary Master:");
        if let Some(disk) = IdeDisk::new(busmaster, 0x1F0, 0x3F4, 0xE, true) {
            ret.push(box disk);
        }
        debugln!("");

        debug!("Primary Slave:");
        if let Some(disk) = IdeDisk::new(busmaster, 0x1F0, 0x3F4, 0xE, false) {
            ret.push(box disk);
        }
        debugln!("");

        debug!("Secondary Master:");
        if let Some(disk) = IdeDisk::new(busmaster + 8, 0x170, 0x374, 0xF, true) {
            ret.push(box disk);
        }
        debugln!("");

        debug!("Secondary Slave:");
        if let Some(disk) = IdeDisk::new(busmaster + 8, 0x170, 0x374, 0xF, false) {
            ret.push(box disk);
        }
        debugln!("");

        ret
    }
}

/// A disk (data storage)
pub struct IdeDisk {
    buscmd: Pio<u8>,
    bussts: Pio<u8>,
    prdt: Prdt,
    data: Pio<u16>,
    error: ReadOnly<u8, Pio<u8>>,
    seccount: Pio<u8>,
    sector0: Pio<u8>,
    sector1: Pio<u8>,
    sector2: Pio<u8>,
    devsel: Pio<u8>,
    sts: ReadOnly<u8, Pio<u8>>,
    cmd: WriteOnly<u8, Pio<u8>>,
    alt_sts: ReadOnly<u8, Pio<u8>>,
    irq: u8,
    master: bool,
}

impl IdeDisk {
    pub fn new(busmaster: u16, base: u16, ctrl: u16, irq: u8, master: bool) -> Option<Self> {
        let mut ret = IdeDisk {
            buscmd: Pio::new(busmaster),
            bussts: Pio::new(busmaster + 2),
            prdt: Prdt::new(busmaster + 4),
            data: Pio::new(base),
            error: ReadOnly::new(Pio::new(base + 1)),
            seccount: Pio::new(base + 2),
            sector0: Pio::new(base + 3),
            sector1: Pio::new(base + 4),
            sector2: Pio::new(base + 5),
            devsel: Pio::new(base + 6),
            sts: ReadOnly::new(Pio::new(base + 7)),
            cmd: WriteOnly::new(Pio::new(base + 7)),
            alt_sts: ReadOnly::new(Pio::new(ctrl + 2)),
            irq: irq,
            master: master,
        };

        if unsafe { ret.identify() } {
            Some(ret)
        } else {
            None
        }
    }

    unsafe fn ide_poll(&self, check_error: bool) -> u8 {
        while self.alt_sts.readf(ATA_SR_BSY) {}

        if check_error {
            let state = self.alt_sts.read();
            if state & ATA_SR_ERR == ATA_SR_ERR {
                return 2;
            }
            if state & ATA_SR_DF == ATA_SR_DF {
                return 1;
            }
            if !(state & ATA_SR_DRQ == ATA_SR_DRQ) {
                return 3;
            }
        }

        0
    }

    pub fn ata(&mut self, cmd: u8, block: u64, len: u16) {
        while self.alt_sts.readf(ATA_SR_BSY) {}

        self.devsel.write(if self.master {
            0b11100000
        } else {
            0b11110000
        });

        self.alt_sts.read();
        self.alt_sts.read();
        self.alt_sts.read();
        self.alt_sts.read();

        while self.alt_sts.readf(ATA_SR_BSY) {}

        /*self.seccount.write((len >> 8) as u8);
        self.sector0.write((block >> 24) as u8);
        self.sector1.write((block >> 32) as u8);
        self.sector2.write((block >> 40) as u8);*/

        self.seccount.write(len as u8);
        self.sector0.write(block as u8);
        self.sector1.write((block >> 8) as u8);
        self.sector2.write((block >> 16) as u8);

        self.cmd.write(cmd);
    }

    /// Identify
    pub unsafe fn identify(&mut self) -> bool {
        if self.alt_sts.read() == 0xFF {
            debug!(" Floating Bus");

            return false;
        }

        self.ata(ATA_CMD_IDENTIFY, 0, 0);

        let status = self.alt_sts.read();
        debug!(" Status: {:X}", status);

        if status == 0 {
            return false;
        }

        let err = self.ide_poll(true);
        if err > 0 {
            debug!(" Error: {:X}", err);

            return false;
        }

        let mut destination = Memory::<u16>::new(256).unwrap();
        for word in 0..256 {
            destination.write(word, self.data.read());
        }

        debug!(" Serial: ");
        for word in 10..20 {
            let d = destination.read(word);
            let a = ((d >> 8) as u8) as char;
            if a != ' ' && a != '\0' {
                debug!("{}", a);
            }
            let b = (d as u8) as char;
            if b != ' ' && b != '\0' {
                debug!("{}", b);
            }
        }

        debug!(" Firmware: ");
        for word in 23..27 {
            let d = destination.read(word);
            let a = ((d >> 8) as u8) as char;
            if a != ' ' && a != '\0' {
                debug!("{}", a);
            }
            let b = (d as u8) as char;
            if b != ' ' && b != '\0' {
                debug!("{}", b);
            }
        }

        debug!(" Model: ");
        for word in 27..47 {
            let d = destination.read(word);
            let a = ((d >> 8) as u8) as char;
            if a != ' ' && a != '\0' {
                debug!("{}", a);
            }
            let b = (d as u8) as char;
            if b != ' ' && b != '\0' {
                debug!("{}", b);
            }
        }

        let mut sectors = (destination.read(100) as u64) | ((destination.read(101) as u64) << 16) |
                          ((destination.read(102) as u64) << 32) |
                          ((destination.read(103) as u64) << 48);

        if sectors == 0 {
            sectors = (destination.read(60) as u64) | ((destination.read(61) as u64) << 16);
        }

        debug!(" Size: {} MB", (sectors / 2048) as usize);

        true
    }

    unsafe fn ata_pio_small(&mut self,
                            block: u64,
                            sectors: u16,
                            buf: usize,
                            write: bool)
                            -> Result<usize> {
        if buf > 0 {
            self.ata(if write {
                ATA_CMD_WRITE_PIO //_EXT
            } else {
                ATA_CMD_READ_PIO //_EXT
            }, block, sectors);

            for sector in 0..sectors as usize {
                let err = self.ide_poll(true);
                if err > 0 {
                    debugln!("IDE Error: {:X}={:X}", err, self.error.read());
                    return Err(Error::new(EIO));
                }

                if write {
                    for word in 0..256 {
                        self.data.write(ptr::read((buf + sector * 512 + word * 2) as *const u16));
                    }

                    self.cmd.write(ATA_CMD_CACHE_FLUSH_EXT);
                    self.ide_poll(false);
                } else {
                    for word in 0..256 {
                        ptr::write((buf + sector * 512 + word * 2) as *mut u16, self.data.read());
                    }
                }
            }

            Ok(sectors as usize * 512)
        } else {
            debugln!("Invalid request");
            Err(Error::new(EIO))
        }
    }

    fn ata_pio(&mut self, block: u64, sectors: usize, buf: usize, write: bool) -> Result<usize> {
        // debugln!("IDE PIO BLOCK: {} SECTORS: {} BUF: {:X} WRITE: {}", block, sectors, buf, write);

        if buf > 0 && sectors > 0 {
            let mut sector: usize = 0;
            while sectors - sector >= 255 {
                if let Err(err) = unsafe {
                    self.ata_pio_small(block + sector as u64, 255, buf + sector * 512, write)
                } {
                    return Err(err);
                }

                sector += 255;
            }
            if sector < sectors {
                if let Err(err) = unsafe {
                    self.ata_pio_small(block + sector as u64,
                                       (sectors - sector) as u16,
                                       buf + sector * 512,
                                       write)
                } {
                    return Err(err);
                }
            }

            Ok(sectors * 512)
        } else {
            debugln!("Invalid request");
            Err(Error::new(EIO))
        }
    }

    unsafe fn ata_dma_small(&mut self,
                            block: u64,
                            sectors: u16,
                            buf: usize,
                            write: bool)
                            -> Result<usize> {
        if buf > 0 {
            self.buscmd.writef(CMD_ACT, false);

            self.prdt.reg.write(0);

            let status = self.bussts.read();
            self.bussts.write(status);

            let entries = if sectors == 0 {
                512
            } else {
                sectors as usize / 128
            };

            let remainder = (sectors % 128) * 512;

            let mut offset = 0;
            for i in 0..entries {
                self.prdt.mem.write(i,
                                    Prd {
                                        addr: buf as u32 + offset,
                                        size: 0,
                                        rsv: 0,
                                        eot: if i == entries - 1 && remainder == 0 {
                                            PRD_EOT
                                        } else {
                                            0
                                        },
                                    });
                offset += 65536
            }

            if remainder > 0 {
                self.prdt.mem.write(entries,
                                    Prd {
                                        addr: buf as u32 + offset,
                                        size: remainder,
                                        rsv: 0,
                                        eot: PRD_EOT,
                                    });
            }

            self.prdt.reg.write(self.prdt.mem.address() as u32);

            self.buscmd.writef(CMD_DIR, !write);


            self.ata(if write {
                ATA_CMD_WRITE_DMA //_EXT
            } else {
                ATA_CMD_READ_DMA //_EXT
            }, block, sectors);

            self.buscmd.writef(CMD_ACT, true);

            while self.bussts.readf(STS_ACT) && !self.bussts.readf(STS_INT) && !self.bussts.readf(STS_ERR) {}

            self.buscmd.writef(CMD_ACT, false);

            self.prdt.reg.write(0);

            let status = self.bussts.read();
            self.bussts.write(status);

            if status & STS_ERR == STS_ERR {
                debugln!("IDE DMA Read Error");
                return Err(Error::new(EIO));
            }

            Ok(sectors as usize * 512)
        } else {
            debugln!("Invalid request");
            Err(Error::new(EIO))
        }
    }

    fn ata_dma(&mut self, block: u64, sectors: usize, buf: usize, write: bool) -> Result<usize> {
        // debugln!("IDE DMA BLOCK: {} SECTORS: {} BUF: {:X} WRITE: {}", block, sectors, buf, write);

        if buf > 0 && sectors > 0 {
            let mut sector: usize = 0;
            while sectors - sector >= 255 {
                if let Err(err) = unsafe {
                    self.ata_dma_small(block + sector as u64, 255, buf + sector * 512, write)
                } {
                    return Err(err);
                }

                sector += 255;
            }
            if sector < sectors {
                if let Err(err) = unsafe {
                    self.ata_dma_small(block + sector as u64,
                                       (sectors - sector) as u16,
                                       buf + sector * 512,
                                       write)
                } {
                    return Err(err);
                }
            }

            Ok(sectors * 512)
        } else {
            debugln!("Invalid request");
            Err(Error::new(EIO))
        }
    }
}

impl Disk for IdeDisk {
    fn name(&self) -> String {
        format!("IDE {} {}", if self.irq == 0xE {
            "Primary"
        } else {
            "Secondary"
        }, if self.master {
            "Master"
        } else {
            "Slave"
        })
    }

    fn read(&mut self, block: u64, buffer: &mut [u8]) -> Result<usize> {
        self.ata_pio(block, buffer.len() / 512, buffer.as_ptr() as usize, false)
    }

    fn write(&mut self, block: u64, buffer: &[u8]) -> Result<usize> {
        self.ata_pio(block, buffer.len() / 512, buffer.as_ptr() as usize, true)
    }
}
