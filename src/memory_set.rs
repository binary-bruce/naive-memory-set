use alloc::vec::Vec;
use page_table::{
    PTEFlags, PageTable, PageTableEntry, PhysAddr, PhysPageNum, VirtAddr, VirtPageNum, PAGE_SIZE,
};
use xmas_elf::program::Flags;

use crate::MemorySetBuilder;

use super::{map_type::MapType, memory_area::MapArea, MapPermission};
use core::arch::asm;
use riscv::register::satp;

/// memory set structure, controls virtual-memory space
pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

impl MemorySet {
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }

    pub fn token(&self) -> usize {
        self.page_table.token()
    }

    /// Assume that no conflicts.
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission),
            None,
        );
    }

    pub fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data);
        }
        self.areas.push(map_area);
    }

    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some((idx, area)) = self
            .areas
            .iter_mut()
            .enumerate()
            .find(|(_, area)| area.vpn_range.get_start() == start_vpn)
        {
            area.unmap(&mut self.page_table);
            self.areas.remove(idx);
        }
    }

    pub fn recycle_data_pages(&mut self) {
        self.areas.clear();
    }

    pub fn activate(&self) {
        let satp = self.page_table.token();
        unsafe {
            satp::write(satp);
            asm!("sfence.vma");
        }
    }

    pub fn map_trampoline(&mut self, vpn: VirtPageNum, ppn: PhysPageNum) {
        self.page_table.map(vpn, ppn, PTEFlags::R | PTEFlags::X);
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }

    pub fn shrink_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.shrink_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    pub fn append_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.append_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    /// clone the memory set
    pub fn from_existed_user(
        user_space: &Self,
        trampline_start_va: usize,
        trampline_start_pa: usize,
    ) -> Self {
        let mut memory_set = Self::new_bare();

        memory_set.map_trampoline(
            VirtAddr::from(trampline_start_va).into(),
            PhysAddr::from(trampline_start_pa as usize).into(),
        );

        // copy data sections/trap_context/user_stack
        for area in user_space.areas.iter() {
            let new_area = MapArea::from_another(area);
            memory_set.push(new_area, None);
            // copy data from another space
            for vpn in area.vpn_range {
                let src_ppn = user_space.translate(vpn).unwrap().ppn();
                let dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
            }
        }

        memory_set
    }

    /// Include sections in elf and trampoline and TrapContext and user stack,
    /// also returns user_sp and entry point.
    pub fn from_elf(
        elf_data: &[u8],
        trampline_start_va: usize,
        trampline_start_pa: usize,
        trap_cx_start_va: usize,
        user_stack_size: usize,
    ) -> (MemorySet, usize, usize) {
        let mut memory_set_builder =
            MemorySetBuilder::new().map_trampoline(trampline_start_va, trampline_start_pa);

        // map program headers of elf, with U flag
        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let ph_count = elf_header.pt2.ph_count();
        let mut max_end_vpn = VirtPageNum(0);
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == xmas_elf::program::Type::Load {
                let start_va: VirtAddr = (ph.virtual_addr() as usize).into();
                let end_va: VirtAddr = ((ph.virtual_addr() + ph.mem_size()) as usize).into();
                let map_perm = Self::get_map_perm(ph.flags());
                let map_area = MapArea::new(start_va, end_va, MapType::Framed, map_perm);
                max_end_vpn = map_area.vpn_range.get_end();

                memory_set_builder = memory_set_builder.push_framed_with_data(
                    start_va.into(),
                    end_va.into(),
                    map_perm,
                    Some(&elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize]),
                );
            }
        }
        // map user stack with U flags
        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut user_stack_bottom: usize = max_end_va.into();
        // guard page
        user_stack_bottom += PAGE_SIZE;
        let user_stack_top = user_stack_bottom + user_stack_size;

        let rwu = MapPermission::R | MapPermission::W | MapPermission::U;
        let rw = MapPermission::R | MapPermission::W;
        let memory_set = memory_set_builder
            .push_framed(user_stack_bottom, user_stack_top, rwu)
            .push_framed(user_stack_top, user_stack_top, rwu)
            .push_framed(trap_cx_start_va, trampline_start_va, rw)
            .build();

        (
            memory_set,
            user_stack_top,
            elf.header.pt2.entry_point() as usize,
        )
    }

    fn get_map_perm(ph_flags: Flags) -> MapPermission {
        let mut map_perm = MapPermission::U;
        if ph_flags.is_read() {
            map_perm |= MapPermission::R;
        }
        if ph_flags.is_write() {
            map_perm |= MapPermission::W;
        }
        if ph_flags.is_execute() {
            map_perm |= MapPermission::X;
        }

        map_perm
    }
}
