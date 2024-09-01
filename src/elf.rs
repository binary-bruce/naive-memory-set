use page_table::{VirtAddr, VirtPageNum, PAGE_SIZE};
use xmas_elf::program::Flags;

use crate::{MapArea, MapPermission, MapType, MemorySet, MemorySetBuilder};

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
            let map_perm = get_map_perm(ph.flags());
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
