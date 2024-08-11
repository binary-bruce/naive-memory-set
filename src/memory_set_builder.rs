use page_table::{PhysAddr, VirtAddr};

use crate::{MapArea, MapPermission, MapType, MemorySet};

pub struct MemorySetBuilder {
    memory_set: MemorySet,
}

impl MemorySetBuilder {
    pub fn new() -> Self {
        Self {
            memory_set: MemorySet::new_bare(),
        }
    }

    pub fn push_identical(
        mut self,
        start_va: usize,
        end_va: usize,
        map_perm: MapPermission,
    ) -> Self {
        self.memory_set.push(
            MapArea::new(start_va.into(), end_va.into(), MapType::Identical, map_perm),
            None,
        );

        self
    }

    /// push identitical memory area
    pub fn push_framed(mut self, start_va: usize, end_va: usize, map_perm: MapPermission) -> Self {
        self.memory_set.push(
            MapArea::new(start_va.into(), end_va.into(), MapType::Framed, map_perm),
            None,
        );

        self
    }

    /// push framed memory area
    pub fn push_framed_with_data(
        mut self,
        start_va: usize,
        end_va: usize,
        map_perm: MapPermission,
        data: Option<&[u8]>,
    ) -> Self {
        self.memory_set.push(
            MapArea::new(start_va.into(), end_va.into(), MapType::Framed, map_perm),
            data,
        );

        self
    }

    pub fn map_trampoline(mut self, va: usize, pa: usize) -> Self {
        self.memory_set
            .map_trampoline(VirtAddr::from(va).into(), PhysAddr::from(pa).into());
        self
    }

    pub fn build(self) -> MemorySet {
        self.memory_set
    }
}
