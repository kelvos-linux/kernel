use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverDomainKind {
    Kernel,
    Storage,
    Network,
    User,
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverDomain {
    pub id: u32,
    pub name: &'static str,
    pub kind: DriverDomainKind,
    pub isolated: bool,
}

impl DriverDomain {
    pub const fn new(id: u32, name: &'static str, kind: DriverDomainKind, isolated: bool) -> Self {
        Self {
            id,
            name,
            kind,
            isolated,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    InvalidSector,
    DeviceNotFound,
    AccessDenied,
    NotSupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockOperation {
    Read,
    Write,
    Flush,
}

pub trait BlockDevice {
    fn device_id(&self) -> u32;
    fn read_sector(&self, sector: u64, buffer: &mut [u8]) -> Result<(), StorageError>;
    fn write_sector(&self, sector: u64, buffer: &[u8]) -> Result<(), StorageError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageRequest {
    pub device_id: u32,
    pub sector: u64,
    pub len: usize,
    pub operation: BlockOperation,
}

#[derive(Clone, Debug)]
pub struct NvmeController {
    pub id: u32,
    pub name: &'static str,
    pub sectors: u64,
    pub logical_block_size: u32,
    pub isolated: bool,
}

impl NvmeController {
    pub const fn new(id: u32, name: &'static str, sectors: u64, logical_block_size: u32, isolated: bool) -> Self {
        Self {
            id,
            name,
            sectors,
            logical_block_size,
            isolated,
        }
    }
}

impl BlockDevice for NvmeController {
    fn device_id(&self) -> u32 {
        self.id
    }

    fn read_sector(&self, sector: u64, buffer: &mut [u8]) -> Result<(), StorageError> {
        if sector >= self.sectors {
            return Err(StorageError::InvalidSector);
        }
        if buffer.len() < self.logical_block_size as usize {
            return Err(StorageError::InvalidSector);
        }
        let _ = sector;
        let _ = buffer;
        Ok(())
    }

    fn write_sector(&self, sector: u64, buffer: &[u8]) -> Result<(), StorageError> {
        if sector >= self.sectors {
            return Err(StorageError::InvalidSector);
        }
        if buffer.len() < self.logical_block_size as usize {
            return Err(StorageError::InvalidSector);
        }
        let _ = sector;
        let _ = buffer;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct StorageRuntime {
    devices: Vec<NvmeController>,
    domains: Vec<DriverDomain>,
}

impl StorageRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_device(&mut self, controller: NvmeController) {
        self.devices.push(controller);
    }

    pub fn register_domain(&mut self, domain: DriverDomain) {
        self.domains.push(domain);
    }

    pub fn validate_device_access(&self, device_id: u32, domain_id: u32) -> Result<(), StorageError> {
        let device = self.devices.iter().find(|d| d.id == device_id).ok_or(StorageError::DeviceNotFound)?;
        let domain = self.domains.iter().find(|d| d.id == domain_id).ok_or(StorageError::AccessDenied)?;

        if device.isolated && !domain.isolated {
            return Err(StorageError::AccessDenied);
        }

        if domain.kind == DriverDomainKind::Kernel || domain.kind == DriverDomainKind::Storage {
            return Ok(());
        }

        if device.isolated {
            return Err(StorageError::AccessDenied);
        }

        Ok(())
    }

    pub fn dispatch_request(&self, domain_id: u32, request: StorageRequest) -> Result<(), StorageError> {
        self.validate_device_access(request.device_id, domain_id)?;

        let device = self.devices.iter().find(|d| d.id == request.device_id).ok_or(StorageError::DeviceNotFound)?;
        if request.sector >= device.sectors {
            return Err(StorageError::InvalidSector);
        }

        match request.operation {
            BlockOperation::Read | BlockOperation::Write => {
                if request.len == 0 || request.len > device.logical_block_size as usize {
                    return Err(StorageError::InvalidSector);
                }
            }
            BlockOperation::Flush => {
                // Flush is allowed to pass the same access check; no additional device I/O is
                // performed in this lightweight runtime because hardware access is behind the
                // device-driver boundary.
            }
        }

        Ok(())
    }
}

pub static STORAGE: spin::Lazy<spin::Mutex<StorageRuntime>> = spin::Lazy::new(|| {
    spin::Mutex::new(StorageRuntime::new())
});

pub fn init() {
    let mut storage = STORAGE.lock();
    storage.register_device(NvmeController::new(0, "nvme0", 1 << 20, 4096, true));
    storage.register_domain(DriverDomain::new(0, "kernel-core", DriverDomainKind::Kernel, true));
    storage.register_domain(DriverDomain::new(1, "storage-stack", DriverDomainKind::Storage, true));
    storage.register_domain(DriverDomain::new(2, "user-domain", DriverDomainKind::User, false));
}
