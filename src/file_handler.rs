use std::fmt::Arguments;
use std::fs::File;
use std::io::{
    BorrowedCursor, BufWriter, Bytes, Chain, IoSlice, IoSliceMut, Read, Result, Seek, SeekFrom,
    Take, Write,
};

pub struct FileHandler {
    file: Option<std::fs::File>,
    pub offset: i64, // todo: should always be positive (maybe change type)
    write: Option<BufWriter<std::fs::File>>,
}

// todo: expose buf write size to user later
impl FileHandler {
    pub fn new(file: std::fs::File, offset: i64, use_write_buffer: bool) -> FileHandler {
        if use_write_buffer {
            FileHandler {
                file: None,
                offset: offset,
                write: Some(BufWriter::with_capacity(4096 * 64, file)),
            }
        } else {
            FileHandler {
                file: Some(file),
                offset: offset,
                write: None,
            }
        }
    }
    pub fn sync_all(&self) -> Result<()> {
        if self.file.is_some() {
            let file = self.file.as_ref().expect("sync_all unexpectedly failed!");
            file.sync_all()
        } else {
            let writer = self.write.as_ref().expect("sync_all unexpectedly failed!");
            writer.get_ref().sync_all()
        }
    }
}

impl Seek for FileHandler {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        if self.file.is_some() {
            self.file
                .as_mut()
                .expect("Seek unexpectedly failed!")
                .seek(pos)
        } else {
            self.write
                .as_mut()
                .expect("Seek unexpectedly failed!")
                .seek(pos)
        }
    }
}

impl Read for FileHandler {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_vectored(bufs)
    }
    fn is_read_vectored(&self) -> bool {
        self.file
            .as_ref()
            .expect("Read called when not expected!")
            .is_read_vectored()
    }
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_to_end(buf)
    }
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_to_string(buf)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_exact(buf)
    }
    fn read_buf(&mut self, buf: BorrowedCursor<'_>) -> Result<()> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_buf(buf)
    }
    fn read_buf_exact(&mut self, cursor: BorrowedCursor<'_>) -> Result<()> {
        self.file
            .as_mut()
            .expect("Read called when not expected!")
            .read_buf_exact(cursor)
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
        match self.write.as_mut() {
            Some(writer) => writer.write(buf),
            None => self.file.as_mut().expect("write failed").write(buf),
        }
    }
    fn flush(&mut self) -> Result<()> {
        match self.write.as_mut() {
            Some(writer) => writer.flush(),
            None => self.file.as_mut().expect("write failed").flush(),
        }
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize> {
        match self.write.as_mut() {
            Some(writer) => writer.write_vectored(bufs),
            None => self
                .file
                .as_mut()
                .expect("write failed")
                .write_vectored(bufs),
        }
    }
    fn is_write_vectored(&self) -> bool {
        match self.write.as_ref() {
            Some(writer) => writer.is_write_vectored(),
            None => self
                .file
                .as_ref()
                .expect("write failed")
                .is_write_vectored(),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        match self.write.as_mut() {
            Some(writer) => writer.write_all(buf),
            None => self.file.as_mut().expect("write failed").write_all(buf),
        }
    }

    fn write_all_vectored(&mut self, bufs: &mut [IoSlice<'_>]) -> Result<()> {
        match self.write.as_mut() {
            Some(writer) => writer.write_all_vectored(bufs),
            None => self
                .file
                .as_mut()
                .expect("write failed")
                .write_all_vectored(bufs),
        }
    }

    fn write_fmt(&mut self, fmt: Arguments<'_>) -> Result<()> {
        match self.write.as_mut() {
            Some(writer) => writer.write_fmt(fmt),
            None => self.file.as_mut().expect("write failed").write_fmt(fmt),
        }
    }

    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }
}
