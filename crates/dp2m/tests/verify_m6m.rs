mod common;

use std::time::Instant;

use dp2m_emu::verify::{Build, run_suites_with_artifacts};

fn check_labels_png(png: &[u8]) {
    fn be32(bytes: &[u8]) -> u32 {
        u32::from_be_bytes(bytes[..4].try_into().unwrap())
    }

    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut chunks = &png[8..];
    let mut dimensions = None;
    let mut idat = Vec::new();
    while !chunks.is_empty() {
        let len = be32(chunks) as usize;
        let data = &chunks[8..8 + len];
        match &chunks[4..8] {
            b"IHDR" => {
                assert!(dimensions.is_none() && idat.is_empty());
                assert_eq!(data.len(), 13);
                let (width, height) = (be32(data) as usize, be32(&data[4..]) as usize);
                assert!(width > 0 && height > 0);
                assert_eq!(&data[8..], &[8, 2, 0, 0, 0]);
                dimensions = Some((width, height));
            }
            b"IDAT" => {
                assert!(dimensions.is_some());
                idat.extend_from_slice(data);
            }
            b"IEND" => {
                assert_eq!(len, 0);
                assert_eq!(&chunks[8..], &0xAE42_6082u32.to_be_bytes());
                break;
            }
            other => panic!("unexpected PNG chunk: {other:?}"),
        }
        chunks = &chunks[12 + len..];
    }
    assert_eq!(&chunks[4..8], b"IEND");
    assert_eq!(&idat[..2], &[0x78, 0x01]);
    assert!(idat.len() > 64 && png.len() < 512 * 1024, "expected compact compressed PNG, got {} B", png.len());
    let (width, height) = dimensions.unwrap();
    assert_eq!((width, height), (2760, 1044));
}

#[test]
fn native_m6m_verify_all_suites() {
    common::with_fresh_build(|out, root| {
        let fw = root.join("data/DP2MV105.bin");
        let t0 = Instant::now();
        let build = Build::load(out, &fw).expect("load fresh build artifacts");
        let artifacts = out.join("artifacts");
        let results = run_suites_with_artifacts(&build, &[], false, Some(&artifacts));
        let elapsed = t0.elapsed();

        let mut total_pass = 0;
        let mut total_fail = 0;
        for s in &results {
            assert!(!s.skipped, "suite {} unexpectedly skipped", s.name);
            println!("  {:<7} {} pass, {} fail", s.name, s.passed, s.failed);
            for f in &s.fails {
                eprintln!("    FAIL [{}] {f}", s.name);
            }
            total_pass += s.passed;
            total_fail += s.failed;
        }
        println!("native Rust verify: {total_pass} pass, {total_fail} fail in {:.2}s", elapsed.as_secs_f64());
        assert_eq!(total_fail, 0, "native M6M verifier reported failures");
        assert!(total_pass >= 490, "expected ~491 checks, got {total_pass}");
        check_labels_png(&std::fs::read(artifacts.join("labels.png")).expect("read label contact sheet"));
    });
}
