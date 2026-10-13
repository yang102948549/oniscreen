//! 화면 녹화(MP4/H.264). Windows Media Foundation Sink Writer로 인코딩한다.
//!
//! 프레임 소스는 호출하는 쪽(확대 스레드)이 만든다. 확대 창과 필기 오버레이는 확대 루프가 자기 화면을 다시
//! 캡처하지 않도록 캡처에서 제외되어 있으므로, 화면 캡처 API(WGC 등)로는 확대·필기가 보이지 않는다.
//! 그래서 확대된 프레임을 직접 합성해서 넘기고, 이 모듈은 인코딩과 파일 기록만 맡는다.
//!
//! 인코딩은 전용 스레드에서 한다. 프레임 큐는 작고 가득 차면 프레임을 버린다(UI/확대 스레드를 절대 막지 않음).
//! 마우스·키 입력을 가로채거나 주입하지 않고, 시스템 커서도 건드리지 않는다.
use std::{
    path::PathBuf,
    sync::mpsc::{sync_channel, SyncSender, TrySendError},
    thread::JoinHandle,
    time::Instant,
};

use windows::{
    core::{Error, Result, HSTRING},
    Win32::{
        Media::MediaFoundation::*,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

pub const FPS: u32 = 30;

/// 인코더에 넘기는 한 프레임: BGRA, 위→아래, `w*h*4` 바이트.
pub struct Frame {
    pub bgra: Vec<u8>,
    /// 녹화 시작 후 경과 시간(100ns 단위)
    pub time_100ns: i64,
}

pub struct Recorder {
    tx: Option<SyncSender<Frame>>,
    worker: Option<JoinHandle<std::result::Result<(), String>>>,
    started: Instant,
    pub path: PathBuf,
    pub dropped: u32,
}

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

impl Recorder {
    /// `w`,`h`는 짝수여야 한다(H.264). 호출하는 쪽에서 맞춰서 넘긴다.
    pub fn start(path: PathBuf, w: i32, h: i32) -> std::result::Result<Self, String> {
        if w < 16 || h < 16 || w % 2 != 0 || h % 2 != 0 || w > 8192 || h > 8192 {
            return Err(format!("지원하지 않는 녹화 크기: {w}x{h}"));
        }
        let (tx, rx) = sync_channel::<Frame>(4);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let p = path.clone();
        let worker = std::thread::Builder::new()
            .name("oni-recorder".into())
            .spawn(move || -> std::result::Result<(), String> {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    let r = encode_loop(&p, w, h, rx, &ready_tx);
                    CoUninitialize();
                    r.map_err(|e| e.to_string())
                }
            })
            .map_err(|e| e.to_string())?;
        // 인코더 준비(파일 생성·코덱 협상)가 끝나기를 짧게 기다린다. 실패하면 이유를 그대로 돌려준다.
        match ready_rx.recv_timeout(std::time::Duration::from_secs(8)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = worker.join();
                return Err(e);
            }
            Err(_) => return Err("인코더 준비 시간 초과".into()),
        }
        Ok(Self { tx: Some(tx), worker: Some(worker), started: Instant::now(), path, dropped: 0 })
    }

    pub fn elapsed_secs(&self) -> u32 {
        self.started.elapsed().as_secs() as u32
    }

    pub fn elapsed_100ns(&self) -> i64 {
        (self.started.elapsed().as_nanos() / 100) as i64
    }

    /// 프레임을 큐에 넣는다. 큐가 가득 차면 버리고 `false`.
    pub fn push(&mut self, bgra: Vec<u8>) -> bool {
        let time_100ns = self.elapsed_100ns();
        let Some(tx) = &self.tx else { return false };
        match tx.try_send(Frame { bgra, time_100ns }) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// 남은 프레임을 기록하고 파일을 닫는다.
    pub fn finish(mut self) -> std::result::Result<PathBuf, String> {
        self.tx.take(); // 채널을 닫으면 인코더가 마무리한다
        let r = match self.worker.take() {
            Some(h) => h.join().map_err(|_| "인코더 스레드가 비정상 종료했습니다".to_string())?,
            None => Ok(()),
        };
        r.map(|_| self.path.clone())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

unsafe fn encode_loop(
    path: &std::path::Path,
    w: i32,
    h: i32,
    rx: std::sync::mpsc::Receiver<Frame>,
    ready: &std::sync::mpsc::Sender<std::result::Result<(), String>>,
) -> Result<()> {
    let setup = (|| -> Result<(IMFSinkWriter, u32)> {
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.ok_or_else(Error::empty)?;
        // 하드웨어 인코더가 있으면 쓰되, 없으면 소프트웨어 MFT로 대체된다.
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attrs)?;

        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        // 비트레이트: 해상도에 비례(1080p ≈ 8Mbps), 수업 녹화에 충분하고 파일이 과하게 커지지 않는다.
        let bitrate = ((w as u64 * h as u64 * 4) as u32).clamp(2_000_000, 24_000_000);
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, pack(w as u32, h as u32))?;
        out.SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        let idx = writer.AddStream(&out)?;

        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(w as u32, h as u32))?;
        inp.SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))?;
        inp.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        // 양수 stride = 위→아래(top-down) 배치
        inp.SetUINT32(&MF_MT_DEFAULT_STRIDE, (w * 4) as u32)?;
        writer.SetInputMediaType(idx, &inp, None)?;
        writer.BeginWriting()?;
        Ok((writer, idx))
    })();

    let (writer, idx) = match setup {
        Ok(v) => {
            let _ = ready.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready.send(Err(format!("인코더를 시작하지 못했습니다: {e}")));
            let _ = MFShutdown();
            return Err(e);
        }
    };

    let frame_len = (w as usize) * (h as usize) * 4;
    let dur = 10_000_000i64 / FPS as i64;
    let mut write_err: Option<Error> = None;
    while let Ok(f) = rx.recv() {
        if f.bgra.len() != frame_len || write_err.is_some() {
            continue;
        }
        let r = (|| -> Result<()> {
            let buf = MFCreateMemoryBuffer(frame_len as u32)?;
            let mut p: *mut u8 = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(f.bgra.as_ptr(), p, frame_len);
            buf.Unlock()?;
            buf.SetCurrentLength(frame_len as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(f.time_100ns)?;
            sample.SetSampleDuration(dur)?;
            writer.WriteSample(idx, &sample)
        })();
        if let Err(e) = r {
            write_err = Some(e);
        }
    }
    let fin = writer.Finalize();
    let _ = MFShutdown();
    match (write_err, fin) {
        (Some(e), _) => Err(e),
        (None, r) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 합성 프레임만 사용한다(실제 화면 캡처 없음). 1초 분량을 인코딩해 유효한 MP4가 나오는지 확인한다.
    #[test]
    fn encodes_synthetic_frames_to_mp4() {
        let (w, h) = (320, 180);
        let path = std::env::temp_dir().join(format!("oniscreen-test-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let rec = Recorder::start(path.clone(), w, h).expect("인코더 시작");
        for i in 0..30u32 {
            let mut px = vec![0u8; (w * h * 4) as usize];
            for (n, p) in px.chunks_exact_mut(4).enumerate() {
                let x = (n as i32 % w) as u32;
                p.copy_from_slice(&[(x + i * 8) as u8, (i * 8) as u8, 200, 255]);
            }
            // 테스트에서는 실제 시간 대신 프레임 번호로 타임스탬프를 만든다.
            let tx = rec.tx.as_ref().unwrap();
            tx.send(Frame { bgra: px, time_100ns: i as i64 * 10_000_000 / FPS as i64 }).unwrap();
        }
        let out = rec.finish().expect("마무리");
        let bytes = std::fs::read(&out).expect("파일 읽기");
        let _ = std::fs::remove_file(&out);
        assert!(bytes.len() > 1000, "너무 작다: {}", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp", "MP4 헤더가 아니다");
    }

    #[test]
    fn rejects_odd_sizes() {
        assert!(Recorder::start(std::env::temp_dir().join("x.mp4"), 321, 180).is_err());
    }
}
