use super::topology::MAX_CPUS;
use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuMask([u64; MAX_CPUS.div_ceil(u64::BITS as usize)]);

impl CpuMask {
    pub const fn empty() -> Self {
        Self([0; MAX_CPUS.div_ceil(u64::BITS as usize)])
    }

    pub fn all(topology_cpu_count: usize) -> Self {
        let mut mask = Self::empty();
        for cpu_id in 0..topology_cpu_count.min(MAX_CPUS) {
            mask.set(cpu_id, true);
        }
        mask
    }

    pub fn set(&mut self, cpu_id: usize, enabled: bool) {
        assert!(cpu_id < MAX_CPUS, "logical CPU id is out of range");
        let word = cpu_id / u64::BITS as usize;
        let bit = 1u64 << (cpu_id % u64::BITS as usize);
        if enabled {
            self.0[word] |= bit;
        } else {
            self.0[word] &= !bit;
        }
    }

    pub fn contains(&self, cpu_id: usize) -> bool {
        cpu_id < MAX_CPUS
            && self.0[cpu_id / u64::BITS as usize] & (1u64 << (cpu_id % u64::BITS as usize)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }
}

impl Default for CpuMask {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Binary for CpuMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, word) in self.0.iter().enumerate().rev() {
            if index != self.0.len() - 1 {
                f.write_str(":")?;
            }
            fmt::Binary::fmt(word, f)?;
        }
        Ok(())
    }
}
