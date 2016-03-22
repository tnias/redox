use core::ops::Deref;
use core_collections::borrow::ToOwned;
use io::{self, Read, Error, Result, Write, Seek, SeekFrom};
use os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use mem;
use path::{PathBuf, Path};
use str;
use string::String;
use sys_common::AsInner;
use vec::Vec;

use system::syscall::{sys_open, sys_dup, sys_close, sys_fpath, sys_ftruncate, sys_read,
              sys_write, sys_lseek, sys_fsync, sys_mkdir, sys_rmdir, sys_stat, sys_unlink};
use system::syscall::{O_RDWR, O_RDONLY, O_WRONLY, O_APPEND, O_CREAT, O_TRUNC, MODE_DIR, MODE_FILE, SEEK_SET, SEEK_CUR, SEEK_END, Stat};

/// A Unix-style file
pub struct File {
    /// The id for the file
    fd: usize,
}

impl File {
    /// Open a new file using a path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<File> {
        let path_str = path.as_ref().as_os_str().as_inner();
        let mut path_c = path_str.to_owned();
        path_c.push_str("\0");
        unsafe {
            sys_open(path_c.as_ptr(), O_RDONLY, 0).map(|fd| File::from_raw_fd(fd) )
        }.map_err(|x| Error::from_sys(x))
    }

    /// Create a new file using a path
    pub fn create<P: AsRef<Path>>(path: P) -> Result<File> {
        let path_str = path.as_ref().as_os_str().as_inner();
        let mut path_c = path_str.to_owned();
        path_c.push_str("\0");
        unsafe {
            sys_open(path_c.as_ptr(), O_CREAT | O_RDWR | O_TRUNC, 0).map(|fd| File::from_raw_fd(fd) )
        }.map_err(|x| Error::from_sys(x))
    }

    /// Duplicate the file
    pub fn dup(&self) -> Result<File> {
        sys_dup(self.fd).map(|fd| unsafe { File::from_raw_fd(fd) }).map_err(|x| Error::from_sys(x))
    }

    /// Get the canonical path of the file
    pub fn path(&self) -> Result<PathBuf> {
        let mut buf: [u8; 4096] = [0; 4096];
        match sys_fpath(self.fd, &mut buf) {
            Ok(count) => Ok(PathBuf::from(unsafe { String::from_utf8_unchecked(Vec::from(&buf[0..count])) })),
            Err(err) => Err(Error::from_sys(err)),
        }
    }

    /// Flush the file data and metadata
    pub fn sync_all(&mut self) -> Result<()> {
        sys_fsync(self.fd).and(Ok(())).map_err(|x| Error::from_sys(x))
    }

    /// Flush the file data
    pub fn sync_data(&mut self) -> Result<()> {
        sys_fsync(self.fd).and(Ok(())).map_err(|x| Error::from_sys(x))
    }

    /// Truncates the file
    pub fn set_len(&mut self, size: u64) -> Result<()> {
        sys_ftruncate(self.fd, size as usize).and(Ok(())).map_err(|x| Error::from_sys(x))
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        File {
            fd: fd
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.fd;
        mem::forget(self);
        fd
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys_read(self.fd, buf).map_err(|x| Error::from_sys(x))
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys_write(self.fd, buf).map_err(|x| Error::from_sys(x))
    }

    // TODO buffered fs
    fn flush(&mut self) -> Result<()> { Ok(()) }
}

impl Seek for File {
    /// Seek a given position
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let (whence, offset) = match pos {
            SeekFrom::Start(offset) => (SEEK_SET, offset as isize),
            SeekFrom::Current(offset) => (SEEK_CUR, offset as isize),
            SeekFrom::End(offset) => (SEEK_END, offset as isize),
        };

        sys_lseek(self.fd, offset, whence).map(|position| position as u64).map_err(|x| Error::from_sys(x))
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys_close(self.fd);
    }
}

pub struct FileType {
    dir: bool,
    file: bool,
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.dir
    }

    pub fn is_file(&self) -> bool {
        self.file
    }
}

pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    create: bool,
    truncate: bool,
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            create: false,
            truncate: false,
        }
    }

    pub fn read(&mut self, read: bool) -> &mut OpenOptions {
        self.read = read;
        self
    }

    pub fn write(&mut self, write: bool) -> &mut OpenOptions {
        self.write = write;
        self
    }

    pub fn append(&mut self, append: bool) -> &mut OpenOptions {
        self.append = append;
        self
    }

    pub fn create(&mut self, create: bool) -> &mut OpenOptions {
        self.create = create;
        self
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut OpenOptions {
        self.truncate = truncate;
        self
    }

    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<File> {
        let mut flags = 0;

        if self.read && self.write {
            flags |= O_RDWR;
        } else if self.read {
            flags |= O_RDONLY;
        } else if self.write {
            flags |= O_WRONLY;
        }

        if self.append {
            flags |= O_APPEND;
        }

        if self.create {
            flags |= O_CREAT;
        }

        if self.truncate {
            flags |= O_TRUNC;
        }

        let path_str = path.as_ref().as_os_str().as_inner();
        let mut path_c = path_str.to_owned();
        path_c.push_str("\0");
        unsafe {
            sys_open(path_c.as_ptr(), flags, 0).map(|fd| File::from_raw_fd(fd))
        }.map_err(|x| Error::from_sys(x))
    }
}

pub struct Metadata {
    stat: Stat
}

impl Metadata {
    pub fn file_type(&self) -> FileType {
        FileType {
            dir: self.stat.st_mode & MODE_DIR == MODE_DIR,
            file: self.stat.st_mode & MODE_FILE == MODE_FILE
        }
    }

    pub fn is_dir(&self) -> bool {
        self.stat.st_mode & MODE_DIR == MODE_DIR
    }

    pub fn is_file(&self) -> bool {
        self.stat.st_mode & MODE_FILE == MODE_FILE
    }

    pub fn len(&self) -> u64 {
        self.stat.st_size
    }
}

pub struct DirEntry {
    path: String,
    dir: bool,
    file: bool,
}

impl DirEntry {
    pub fn file_name(&self) -> &Path {
        unsafe { mem::transmute(self.path.deref()) }
    }

    pub fn file_type(&self) -> Result<FileType> {
        Ok(FileType {
            dir: self.dir,
            file: self.file,
        })
    }

    pub fn path(&self) -> PathBuf {
        PathBuf::from(self.path.clone())
    }
}

pub struct ReadDir {
    file: File,
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry>;
    fn next(&mut self) -> Option<Result<DirEntry>> {
        let mut path = String::new();
        let mut buf: [u8; 1] = [0; 1];
        loop {
            match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(count) => {
                    if buf[0] == 10 {
                        break;
                    } else {
                        path.push_str(unsafe { str::from_utf8_unchecked(&buf[..count]) });
                    }
                }
                Err(_err) => break,
            }
        }
        if path.is_empty() {
            None
        } else {
            let dir = path.ends_with('/');
            if dir {
                path.pop();
            }
            Some(Ok(DirEntry {
                path: path,
                dir: dir,
                file: !dir,
            }))
        }
    }
}

/// Find the canonical path of a file
pub fn canonicalize<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
    match File::open(path) {
        Ok(file) => {
            match file.path() {
                Ok(realpath) => Ok(realpath),
                Err(err) => Err(err)
            }
        },
        Err(err) => Err(err)
    }
}

pub fn metadata<P: AsRef<Path>>(path: P) -> Result<Metadata> {
    let mut stat = Stat {
        st_mode: 0,
        st_size: 0
    };
    let path_str = path.as_ref().as_os_str().as_inner();
    let mut path_c = path_str.to_owned();
    path_c.push_str("\0");
    unsafe {
        try!(sys_stat(path_c.as_ptr(), &mut stat).map_err(|x| Error::from_sys(x)));
    }
    Ok(Metadata {
        stat: stat
    })
}

/// Create a new directory, using a path
/// The default mode of the directory is 744
pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_str = path.as_ref().as_os_str().as_inner();
    let mut path_c = path_str.to_owned();
    path_c.push_str("\0");
    unsafe {
        sys_mkdir(path_c.as_ptr(), 755).and(Ok(())).map_err(|x| Error::from_sys(x))
    }
}

pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<u64> {
    let mut infile = try!(File::open(from));
    let mut outfile = try!(File::create(to));
    io::copy(&mut infile, &mut outfile)
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<()> {
    try!(copy(Path::new(from.as_ref()), to));
    remove_file(from)
}

pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<ReadDir> {
    File::open(path).map(|file| ReadDir { file: file })
}

pub fn remove_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_str = path.as_ref().as_os_str().as_inner();
    let mut path_c = path_str.to_owned();
    path_c.push_str("\0");
    unsafe {
        sys_rmdir(path_c.as_ptr()).and(Ok(()))
    }.map_err(|x| Error::from_sys(x))
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<()> {
    let path_str = path.as_ref().as_os_str().as_inner();
    let mut path_c = path_str.to_owned();
    path_c.push_str("\0");
    unsafe {
        sys_unlink(path_c.as_ptr()).and(Ok(()))
    }.map_err(|x| Error::from_sys(x))
}
