use alloc::{collections::VecDeque, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessRights(u8);

impl AccessRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const KERNEL: Self = Self(1 << 3);

    pub const fn allows(self, required: Self) -> bool {
        (self.0 & required.0) == required.0
    }
}

impl core::ops::BitOr for AccessRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityError {
    InvalidRange,
    InvalidSyscall,
    CapabilityDenied,
    MemoryViolation,
    InvalidTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    pub object_id: u32,
    pub rights: AccessRights,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct CapabilityTable {
    entries: Vec<Capability>,
}

impl CapabilityTable {
    pub fn grant(&mut self, object_id: u32, rights: AccessRights) {
        self.entries.push(Capability { object_id, rights });
    }

    pub fn revoke(&mut self, object_id: u32) {
        self.entries.retain(|entry| entry.object_id != object_id);
    }

    pub fn validate(&self, object_id: u32, required: AccessRights) -> Result<(), SecurityError> {
        for entry in &self.entries {
            if entry.object_id == object_id && entry.rights.allows(required) {
                return Ok(());
            }
        }
        Err(SecurityError::CapabilityDenied)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub base: usize,
    pub len: usize,
    pub rights: AccessRights,
}

#[derive(Default, Debug)]
pub struct MemoryProtectionManager {
    regions: Vec<MemoryRegion>,
}

impl MemoryProtectionManager {
    pub fn register_region(&mut self, region: MemoryRegion) -> Result<(), SecurityError> {
        if region.len == 0 {
            return Err(SecurityError::InvalidRange);
        }
        self.regions.push(region);
        Ok(())
    }

    pub fn validate_range(
        &self,
        ptr: usize,
        len: usize,
        required: AccessRights,
    ) -> Result<(), SecurityError> {
        if len == 0 {
            return Ok(());
        }

        let end = ptr.checked_add(len).ok_or(SecurityError::InvalidRange)?;
        for region in &self.regions {
            let region_end = region
                .base
                .checked_add(region.len)
                .ok_or(SecurityError::InvalidRange)?;
            if ptr >= region.base && end <= region_end {
                if region.rights.allows(required) {
                    return Ok(());
                }
                return Err(SecurityError::MemoryViolation);
            }
        }

        Err(SecurityError::MemoryViolation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: &'static str,
    pub address_space_id: u32,
    pub is_isolated: bool,
    pub capabilities: CapabilityTable,
}

impl Process {
    pub fn new(pid: u32, name: &'static str, address_space_id: u32) -> Self {
        Self {
            pid,
            name,
            address_space_id,
            is_isolated: true,
            capabilities: CapabilityTable::default(),
        }
    }

    pub fn grant_capability(&mut self, object_id: u32, rights: AccessRights) {
        self.capabilities.grant(object_id, rights);
    }

    pub fn validate_capability(
        &self,
        object_id: u32,
        rights: AccessRights,
    ) -> Result<(), SecurityError> {
        self.capabilities.validate(object_id, rights)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcMessage {
    pub from_pid: u32,
    pub to_pid: u32,
    pub payload: [u8; 64],
}

#[derive(Clone, Debug)]
pub struct IpcEndpoint {
    pub endpoint_id: u32,
    pub owner_pid: u32,
    pub peer_pid: Option<u32>,
    pub queue: VecDeque<IpcMessage>,
}

#[derive(Default, Debug)]
pub struct SecureIpc {
    endpoints: Vec<IpcEndpoint>,
}

impl SecureIpc {
    pub fn create_endpoint(
        &mut self,
        endpoint_id: u32,
        owner_pid: u32,
    ) -> Result<(), SecurityError> {
        if self
            .endpoints
            .iter()
            .any(|ep| ep.endpoint_id == endpoint_id)
        {
            return Err(SecurityError::InvalidTarget);
        }

        self.endpoints.push(IpcEndpoint {
            endpoint_id,
            owner_pid,
            peer_pid: None,
            queue: VecDeque::new(),
        });
        Ok(())
    }

    pub fn connect(&mut self, endpoint_id: u32, peer_pid: u32) -> Result<(), SecurityError> {
        let endpoint = self
            .endpoints
            .iter_mut()
            .find(|ep| ep.endpoint_id == endpoint_id)
            .ok_or(SecurityError::InvalidTarget)?;
        endpoint.peer_pid = Some(peer_pid);
        Ok(())
    }

    pub fn send(
        &mut self,
        endpoint_id: u32,
        from_pid: u32,
        to_pid: u32,
        payload: [u8; 64],
    ) -> Result<(), SecurityError> {
        let endpoint = self
            .endpoints
            .iter_mut()
            .find(|ep| ep.endpoint_id == endpoint_id)
            .ok_or(SecurityError::InvalidTarget)?;
        if endpoint.owner_pid != from_pid && endpoint.peer_pid != Some(from_pid) {
            return Err(SecurityError::CapabilityDenied);
        }

        endpoint.queue.push_back(IpcMessage {
            from_pid,
            to_pid,
            payload,
        });
        Ok(())
    }

    pub fn recv(&mut self, endpoint_id: u32, pid: u32) -> Result<IpcMessage, SecurityError> {
        let endpoint = self
            .endpoints
            .iter_mut()
            .find(|ep| ep.endpoint_id == endpoint_id)
            .ok_or(SecurityError::InvalidTarget)?;
        if endpoint.peer_pid != Some(pid) && endpoint.owner_pid != pid {
            return Err(SecurityError::CapabilityDenied);
        }
        endpoint
            .queue
            .pop_front()
            .ok_or(SecurityError::InvalidTarget)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyscallKind {
    Read,
    Write,
    MapMemory,
    CreateEndpoint,
    ConnectEndpoint,
    SendIpc,
    ReceiveIpc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallRequest {
    pub pid: u32,
    pub kind: SyscallKind,
    pub object_id: u32,
    pub ptr: usize,
    pub len: usize,
    pub rights: AccessRights,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallFrame {
    pub pid: u32,
    pub syscall_no: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    pub arg5: u64,
}

impl SyscallFrame {
    pub fn request(self) -> Result<SyscallRequest, SecurityError> {
        let kind = match self.syscall_no {
            0 => SyscallKind::Read,
            1 => SyscallKind::Write,
            2 => SyscallKind::MapMemory,
            3 => SyscallKind::CreateEndpoint,
            4 => SyscallKind::ConnectEndpoint,
            5 => SyscallKind::SendIpc,
            6 => SyscallKind::ReceiveIpc,
            _ => return Err(SecurityError::InvalidSyscall),
        };

        Ok(SyscallRequest {
            pid: self.pid,
            kind,
            object_id: self.arg1 as u32,
            ptr: self.arg2 as usize,
            len: self.arg3 as usize,
            rights: match kind {
                SyscallKind::Read => AccessRights::READ,
                SyscallKind::Write => AccessRights::WRITE,
                SyscallKind::MapMemory => AccessRights::READ | AccessRights::WRITE,
                SyscallKind::CreateEndpoint => AccessRights::KERNEL,
                SyscallKind::ConnectEndpoint => AccessRights::READ,
                SyscallKind::SendIpc | SyscallKind::ReceiveIpc => {
                    AccessRights::READ | AccessRights::WRITE
                }
            },
        })
    }
}

#[derive(Default, Debug)]
pub struct SecurityRuntime {
    pub processes: spin::Mutex<Vec<Process>>,
    pub memory_protection: spin::Mutex<MemoryProtectionManager>,
    pub ipc: spin::Mutex<SecureIpc>,
    pub syscall_table: spin::Mutex<CapabilityTable>,
}

impl SecurityRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_process(&self, process: Process) {
        self.processes.lock().push(process);
    }

    pub fn validate_syscall(
        &self,
        process_id: u32,
        object_id: u32,
        rights: AccessRights,
    ) -> Result<(), SecurityError> {
        let processes = self.processes.lock();
        let process = processes
            .iter()
            .find(|p| p.pid == process_id)
            .ok_or(SecurityError::InvalidSyscall)?;
        process.validate_capability(object_id, rights)
    }

    pub fn validate_memory(
        &self,
        ptr: usize,
        len: usize,
        required: AccessRights,
    ) -> Result<(), SecurityError> {
        self.memory_protection
            .lock()
            .validate_range(ptr, len, required)
    }

    pub fn create_ipc_endpoint(
        &self,
        endpoint_id: u32,
        owner_pid: u32,
    ) -> Result<(), SecurityError> {
        self.ipc.lock().create_endpoint(endpoint_id, owner_pid)
    }

    pub fn dispatch_syscall(&self, request: SyscallRequest) -> Result<(), SecurityError> {
        let processes = self.processes.lock();
        let process = processes
            .iter()
            .find(|p| p.pid == request.pid)
            .ok_or(SecurityError::InvalidSyscall)?
            .clone();

        match request.kind {
            SyscallKind::Read => {
                process.validate_capability(request.object_id, request.rights)?;
            }
            SyscallKind::Write => {
                process.validate_capability(request.object_id, request.rights)?;
            }
            SyscallKind::MapMemory => {
                self.validate_memory(request.ptr, request.len, request.rights)?;
            }
            SyscallKind::CreateEndpoint => {
                if request.rights == AccessRights::KERNEL
                    || request.rights.allows(AccessRights::KERNEL)
                {
                    drop(processes);
                    return self.create_ipc_endpoint(request.object_id, request.pid);
                }
                return Err(SecurityError::CapabilityDenied);
            }
            SyscallKind::ConnectEndpoint => {
                process.validate_capability(request.object_id, request.rights)?;
            }
            SyscallKind::SendIpc | SyscallKind::ReceiveIpc => {
                process.validate_capability(request.object_id, request.rights)?;
            }
        }

        Ok(())
    }

    pub fn syscall_read(
        &self,
        pid: u32,
        object_id: u32,
        ptr: usize,
        len: usize,
    ) -> Result<(), SecurityError> {
        self.dispatch_syscall(SyscallRequest {
            pid,
            kind: SyscallKind::Read,
            object_id,
            ptr,
            len,
            rights: AccessRights::READ,
        })
    }

    pub fn syscall_write(
        &self,
        pid: u32,
        object_id: u32,
        ptr: usize,
        len: usize,
    ) -> Result<(), SecurityError> {
        self.dispatch_syscall(SyscallRequest {
            pid,
            kind: SyscallKind::Write,
            object_id,
            ptr,
            len,
            rights: AccessRights::WRITE,
        })
    }

    pub fn handle_syscall(&self, frame: SyscallFrame) -> Result<u64, SecurityError> {
        let request = frame.request()?;
        self.dispatch_syscall(request)?;
        Ok(0)
    }
}

pub static SECURITY: spin::Lazy<SecurityRuntime> = spin::Lazy::new(SecurityRuntime::new);

pub fn init() {
    let mut memory = SECURITY.memory_protection.lock();
    memory
        .register_region(MemoryRegion {
            base: 0x1000,
            len: 0x4000,
            rights: AccessRights::READ | AccessRights::WRITE,
        })
        .ok();

    let mut processes = SECURITY.processes.lock();
    let mut kernel = Process::new(0, "kernel", 0);
    kernel.grant_capability(
        0x1000,
        AccessRights::READ | AccessRights::WRITE | AccessRights::KERNEL,
    );
    kernel.grant_capability(
        1,
        AccessRights::READ | AccessRights::WRITE | AccessRights::KERNEL,
    );
    processes.push(kernel);

    let mut user = Process::new(1, "user", 1);
    user.grant_capability(0x1000, AccessRights::READ);
    user.grant_capability(1, AccessRights::READ | AccessRights::WRITE);
    processes.push(user);

    let mut ipc = SECURITY.ipc.lock();
    ipc.create_endpoint(1, 0).ok();
    ipc.create_endpoint(2, 1).ok();
}
