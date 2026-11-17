#![cfg(target_os = "windows")]

use std::io::{Read, Seek, SeekFrom};

/// Inspect the linked desktop, without starting GPUI or opening user data.
/// A test thread's stack would not catch a regression in the GUI main thread.
#[test]
fn desktop_reserves_enough_main_thread_stack_for_debug_rendering() {
    let mut file = std::fs::File::open(env!("CARGO_BIN_EXE_wisp-gpui")).unwrap();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).unwrap();
    assert_eq!(&dos[..2], b"MZ");
    let pe = u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap());
    file.seek(SeekFrom::Start(pe.into())).unwrap();
    // Only the headers are needed; debug executables can be hundreds of MiB.
    let mut image = [0; 104];
    file.read_exact(&mut image).unwrap();
    assert_eq!(&image[..4], b"PE\0\0");
    let optional = 24;
    let magic = u16::from_le_bytes(image[optional..optional + 2].try_into().unwrap());
    let stack = optional + 72;
    let reserved = match magic {
        0x20b => u64::from_le_bytes(image[stack..stack + 8].try_into().unwrap()),
        0x10b => u32::from_le_bytes(image[stack..stack + 4].try_into().unwrap()).into(),
        _ => panic!("unexpected PE optional header: {magic:#x}"),
    };
    assert!(
        reserved >= 8 * 1024 * 1024,
        "the GPUI main thread needs at least 8 MiB; executable reserves {reserved} bytes"
    );
}
