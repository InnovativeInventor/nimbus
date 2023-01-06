use std::fmt::Arguments;
use std::fs::File;
use std::io::{
    BorrowedCursor, BufWriter, Bytes, Chain, IoSlice, IoSliceMut, Read, Result, Seek, SeekFrom,
    Take, Write,
};

pub struct FileHandler {
    file: std::fs::File,
    pub offset: i64, // todo: should always be positive (maybe change type)
                     // write: BufWriter<std::fs::File>,
}

impl FileHandler {
    pub fn new(file: std::fs::File, offset: i64) -> FileHandler {
        FileHandler { offset, file }
    }
    pub fn sync_all(&self) -> Result<()> {
        self.file.sync_all()
    }
}

impl Seek for FileHandler {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.file.seek(pos)
    }
}

impl Read for FileHandler {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.file.read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize> {
        self.file.read_vectored(bufs)
    }
    fn is_read_vectored(&self) -> bool {
        self.file.is_read_vectored()
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.file.read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        self.file.read_to_string(buf)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.file.read_exact(buf)
    }
    fn read_buf(&mut self, buf: BorrowedCursor<'_>) -> Result<()> {
        self.file.read_buf(buf)
    }
    fn read_buf_exact(&mut self, cursor: BorrowedCursor<'_>) -> Result<()> {
        self.file.read_buf_exact(cursor)
    }
    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }
    fn bytes(self) -> Bytes<Self>
    where
        Self: Sized,
    {
        todo!()
    }
    fn chain<R: Read>(self, next: R) -> Chain<Self, R>
    where
        Self: Sized,
    {
        todo!()
    }
    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl Write for FileHandler {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        self.file.flush()
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize> {
        self.file.write_vectored(bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.file.is_write_vectored()
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.file.write_all(buf)
    }
    fn write_all_vectored(&mut self, bufs: &mut [IoSlice<'_>]) -> Result<()> {
        self.file.write_all_vectored(bufs)
    }

    fn write_fmt(&mut self, fmt: Arguments<'_>) -> Result<()> {
        self.file.write_fmt(fmt)
    }

    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }
}
