mod common;

use std::process::Command;

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
        let build = Build::load(out, &fw).expect("load fresh build artifacts");
        let artifacts = out.join("artifacts");
        let results = run_suites_with_artifacts(&build, &[], false, Some(&artifacts));

        let expected = [("m6m", 241), ("custom", 130), ("labels", 135)];
        assert_eq!(results.len(), expected.len());
        for (s, (name, min_pass)) in results.iter().zip(expected) {
            assert_eq!(s.name, name);
            assert!(!s.skipped, "suite {} unexpectedly skipped", s.name);
            assert_eq!(s.failed, 0, "{name} failures: {:?}", s.fails);
            assert!(s.passed >= min_pass, "{name}: expected at least {min_pass} checks, got {}", s.passed);
        }
        check_labels_png(&std::fs::read(artifacts.join("labels.png")).expect("read label contact sheet"));

        // A default CLI verification must not exit successfully when a suite could not run.
        let syms_path = out.join("m6m/m6m.syms");
        let syms = std::fs::read_to_string(&syms_path).expect("read symbols");
        let mut removed = false;
        let mut incomplete_syms = syms
            .lines()
            .filter(|line| {
                let remove = line.split_whitespace().nth(2) == Some("label_icon");
                removed |= remove;
                !remove
            })
            .collect::<Vec<_>>()
            .join("\n");
        incomplete_syms.push('\n');
        assert!(removed, "test setup must remove label_icon");
        std::fs::write(&syms_path, incomplete_syms).expect("write incomplete symbols");

        let output = Command::new(env!("CARGO_BIN_EXE_dp2m"))
            .current_dir(root)
            .arg("verify")
            .arg(out)
            .arg("--quiet")
            .output()
            .expect("run CLI verifier with incomplete symbols");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "SKIP must fail the CLI:\n{stdout}\n{stderr}");
        assert!(stdout.contains("labels") && stdout.contains("SKIP"), "missing SKIP summary:\n{stdout}");
        assert!(
            stderr.contains("verification incomplete: skipped suites: labels"),
            "missing skipped-suite error:\n{stderr}"
        );
    });
}
