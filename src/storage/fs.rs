use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsNodeType {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub inode: u32,
    pub name: &'static str,
}

#[derive(Clone, Debug)]
pub struct Inode {
    pub id: u32,
    pub name: &'static str,
    pub kind: FsNodeType,
    pub size: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct SimpleFilesystem {
    inodes: Vec<Inode>,
    root: Vec<DirectoryEntry>,
}

impl SimpleFilesystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_file(&mut self, name: &'static str, data: &[u8]) -> u32 {
        let inode_id = self.inodes.len() as u32;
        self.inodes.push(Inode {
            id: inode_id,
            name,
            kind: FsNodeType::File,
            size: data.len(),
            data: data.to_vec(),
        });
        self.root.push(DirectoryEntry { inode: inode_id, name });
        inode_id
    }

    pub fn create_directory(&mut self, name: &'static str) -> u32 {
        let inode_id = self.inodes.len() as u32;
        self.inodes.push(Inode {
            id: inode_id,
            name,
            kind: FsNodeType::Directory,
            size: 0,
            data: Vec::new(),
        });
        self.root.push(DirectoryEntry { inode: inode_id, name });
        inode_id
    }

    pub fn read_file(&self, inode_id: u32) -> Option<&[u8]> {
        self.inodes.iter().find(|inode| inode.id == inode_id).map(|inode| inode.data.as_slice())
    }

    pub fn write_file(&mut self, inode_id: u32, data: &[u8]) -> Result<(), &'static str> {
        let inode = self.inodes.iter_mut().find(|inode| inode.id == inode_id).ok_or("inode not found")?;
        inode.data = data.to_vec();
        inode.size = data.len();
        Ok(())
    }

    pub fn list_root(&self) -> &[DirectoryEntry] {
        &self.root
    }
}

pub static FILESYSTEM: spin::Lazy<spin::Mutex<SimpleFilesystem>> = spin::Lazy::new(|| {
    let mut fs = SimpleFilesystem::new();
    fs.create_directory("/root");
    fs.create_file("boot.cfg", b"kernel=kelv-os\n");
    spin::Mutex::new(fs)
});

pub fn init() {
    let _fs = FILESYSTEM.lock();
}
