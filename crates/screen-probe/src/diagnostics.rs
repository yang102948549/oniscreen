//! Containment build. No capture, magnification, hooks, global keys, or recovery APIs.
use std::ptr::null_mut;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
};

const NOTICE: &str = "화면 멈춤 및 마우스 포인터 소실로 화면 제어 기능을 중단했습니다.\n\n이 버전은 안내 창만 표시합니다. 확대·펜·포인터 효과와 전역 단축키는 동작하지 않습니다.\n\n기존 버전은 다시 실행하지 마세요. 정지 원인은 조사 중이며, 기능 복구가 완료된 버전이 아닙니다.\n\n확인 또는 Alt+F4로 종료할 수 있습니다.";

#[derive(Debug, PartialEq)]
enum Action {
    Notice,
    Help,
    Smoke,
}

fn action(args: &[String]) -> Result<Action, String> {
    match args {
        [] => Ok(Action::Notice),
        [arg] if arg == "--help" => Ok(Action::Help),
        [arg] if arg == "--smoke-test" => Ok(Action::Smoke),
        [arg, number] if arg == "--monitor" => {
            number
                .parse::<usize>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("--monitor requires a positive number")?;
            // Preserve the old launch command without touching any monitor.
            Ok(Action::Notice)
        }
        _ => Err(
            "Containment build: this option is disabled. Use --help. No desktop changes were made."
                .into(),
        ),
    }
}

pub fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match action(&args)? {
        Action::Help => println!(
            "Screen Probe - CONTAINMENT BUILD\nAll screen-control features are disabled.\nNo capture, Magnification, input hooks, synthetic input, cursor changes or global hotkeys.\n--smoke-test: console-only diagnostic; does not initialize the old engine.\n--monitor N: displays the same suspension notice; does not select or change a display.\nLegacy guardian and cursor-recovery commands are disabled."
        ),
        Action::Smoke => println!(
            "PASS: containment entry point. Legacy engine excluded at compile time. No desktop APIs invoked."
        ),
        Action::Notice => {
            eprintln!("{NOTICE}");
            let text: Vec<u16> = NOTICE.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Screen Probe - 기능 중단 안내"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // Ordinary dismissible dialog: no topmost/fullscreen or system-modal flags.
            let result = unsafe {
                MessageBoxW(
                    null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
                )
            };
            if result == 0 {
                return Err("Unable to display suspension notice".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Action, String> {
        action(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn old_internal_commands_cannot_reactivate_engine_or_cursor_apis() {
        for args in [
            vec!["--guardian", "123", "456"],
            vec!["--guardian-test"],
            vec!["--restore-cursor"],
            vec!["--live"],
            vec!["--smoke-test", "--guardian-test"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn supported_launches_only_show_notice_or_console_diagnostic() {
        assert_eq!(parse(&[]), Ok(Action::Notice));
        assert_eq!(parse(&["--monitor", "2"]), Ok(Action::Notice));
        assert_eq!(parse(&["--smoke-test"]), Ok(Action::Smoke));
        assert_eq!(parse(&["--help"]), Ok(Action::Help));
        assert!(parse(&["--monitor", "0"]).is_err());
    }
}
