use crate::{
    capture,
    engine::{Canvas, Frame},
    geometry::{Point, Rect},
    presentation,
    supervisor::{self, Limits, Outcome},
};

pub fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--help" => {
            println!("Screen Probe v2 — isolated zoom and ink\nDefault: supervised control window. No capture until explicitly activated.\n--monitor N: select a monitor (1-based)\n--list-monitors: read monitor list\n--self-test / --smoke-test: fixture/process/hidden-window tests only; no screen capture, cursor changes or global keys\n--render-fixture PATH.bmp: synthetic offscreen image\nCtrl+1 frozen | Ctrl+2 live view/ink | Ctrl+4 pen | Ctrl+5 clear | Ctrl+6/7 zoom | Ctrl+8 stop | Ctrl+9 quit\nCtrl+0 independent emergency exit. Esc returns to control. No input forwarding to underlying apps.");
            Ok(())
        }
        [flag] if flag == "--list-monitors" => {
            for (i, (_, name, r)) in presentation::monitors().iter().enumerate() {
                println!(
                    "{}: {name} {}x{} ({},{})",
                    i + 1,
                    r.width(),
                    r.height(),
                    r.left,
                    r.top
                );
            }
            Ok(())
        }
        [flag] if flag == "--self-test" || flag == "--smoke-test" => {
            capture::self_test()?;
            supervisor::self_test()?;
            let result = supervisor::run(&["--ui-fixture-test".into()], Limits::default(), false)?;
            if result != Outcome::Exited(0) {
                return Err(format!("Hidden UI test failed: {result:?}"));
            }
            println!("PASS: all isolated checks. Real desktop capture/live output and meeting sharing NOT tested.");
            Ok(())
        }
        [flag, path] if flag == "--render-fixture" => {
            let source = Frame::fixture(960, 540)?;
            let mut canvas = Canvas::new(Rect {
                left: 0,
                top: 0,
                right: 960,
                bottom: 540,
            });
            canvas.begin(Point { x: 200, y: 210 });
            canvas.extend(Point { x: 340, y: 320 });
            canvas.extend(Point { x: 680, y: 170 });
            canvas.end();
            canvas.render(&source)?.bmp(std::io::BufWriter::new(
                std::fs::File::create(path).map_err(|e| e.to_string())?,
            ))?;
            println!("Rendered synthetic image: {path}");
            Ok(())
        }
        [flag, mode] if flag == "--fault-worker" => supervisor::fault_worker(mode),
        [flag] if flag == "--job-owner-fixture" => supervisor::job_owner_fixture(),
        [flag, mode] if flag == "--capture-fixture" => capture::fixture_worker(mode),
        [flag] if flag == "--ui-fixture-test" => presentation::run(0, true),
        [flag, index] if flag == "--presentation-worker" => {
            presentation::run(parse_index(index)?, false)
        }
        [flag, left, top, width, height] if flag == "--capture-worker" => {
            let parse = |s: &String| s.parse::<i32>().map_err(|e| e.to_string());
            let (left, top, width, height) =
                (parse(left)?, parse(top)?, parse(width)?, parse(height)?);
            let right = left
                .checked_add(width)
                .ok_or("Capture coordinate overflow")?;
            let bottom = top
                .checked_add(height)
                .ok_or("Capture coordinate overflow")?;
            capture::worker(Rect {
                left,
                top,
                right,
                bottom,
            })
        }
        [] => launch(0),
        [flag, index] if flag == "--monitor" => launch(parse_index(index)?),
        _ => Err("Unknown or retired option. Use --help. No worker started.".into()),
    }
}
fn parse_index(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .ok_or("--monitor requires a positive number".into())
}
fn launch(index: usize) -> Result<(), String> {
    match supervisor::run(&["--presentation-worker".into(),(index+1).to_string()],Limits::default(),true)? {
        Outcome::Exited(0)=>Ok(()),
        Outcome::Exited(code)=>Err(format!("Presentation exited with code {code}")),
        Outcome::Stopped(reason)=>Err(format!("Presentation process tree stopped: {reason}. No cursor restoration is needed; cursor state was never changed.")),
    }
}
