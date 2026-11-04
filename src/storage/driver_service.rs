use alloc::vec::Vec;

use crate::storage::storage::{DriverDomain, DriverDomainKind, STORAGE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverServiceState {
    Ready,
    Isolated,
    Suspended,
}

#[derive(Clone, Debug)]
pub struct DriverServiceDomain {
    pub id: u32,
    pub name: &'static str,
    pub kind: DriverDomainKind,
    pub state: DriverServiceState,
    pub isolated: bool,
    pub allowed_devices: Vec<u32>,
}

impl DriverServiceDomain {
    pub fn new(id: u32, name: &'static str, kind: DriverDomainKind, isolated: bool) -> Self {
        Self {
            id,
            name,
            kind,
            state: DriverServiceState::Ready,
            isolated,
            allowed_devices: Vec::new(),
        }
    }

    pub fn allow_device(&mut self, device_id: u32) {
        if !self.allowed_devices.contains(&device_id) {
            self.allowed_devices.push(device_id);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DriverServiceRuntime {
    domains: Vec<DriverServiceDomain>,
}

impl DriverServiceRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, domain: DriverServiceDomain) {
        self.domains.push(domain);
    }

    pub fn grant_device_access(&mut self, domain_id: u32, device_id: u32) -> bool {
        let Some(domain) = self.domains.iter_mut().find(|d| d.id == domain_id) else {
            return false;
        };

        if !domain.allowed_devices.contains(&device_id) {
            domain.allowed_devices.push(device_id);
        }
        true
    }

    pub fn is_device_allowed(&self, domain_id: u32, device_id: u32) -> bool {
        let domain = self.domains.iter().find(|d| d.id == domain_id);
        match domain {
            Some(d) => d.allowed_devices.contains(&device_id) || d.kind == DriverDomainKind::Kernel,
            None => false,
        }
    }

    pub fn validate_block_request(&self, domain_id: u32, device_id: u32) -> bool {
        if !self.is_device_allowed(domain_id, device_id) {
            return false;
        }

        let Some(domain) = self.domains.iter().find(|d| d.id == domain_id) else {
            return false;
        };

        !domain.isolated || domain.kind == DriverDomainKind::Kernel
    }
}

pub static DRIVER_SERVICES: spin::Lazy<spin::Mutex<DriverServiceRuntime>> =
    spin::Lazy::new(|| spin::Mutex::new(DriverServiceRuntime::new()));

pub fn init() {
    let mut services = DRIVER_SERVICES.lock();
    let mut storage_stack =
        DriverServiceDomain::new(1, "storage-stack", DriverDomainKind::Storage, true);
    storage_stack.allow_device(0);
    services.register(storage_stack);

    let mut user_domain = DriverServiceDomain::new(2, "user-domain", DriverDomainKind::User, false);
    user_domain.allow_device(0);
    services.register(user_domain);

    let mut storage = STORAGE.lock();
    storage.register_domain(DriverDomain::new(
        3,
        "storage-service-domain",
        DriverDomainKind::Storage,
        true,
    ));
}
