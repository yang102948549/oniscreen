# OniScreen

강의·발표용 Windows 화면 확대 · 필기 · 포인터 강조 도구입니다. (Tauri 2 + Rust, 이전 이름: Oni Zoom)

## 기능

- **확대**: 1~6배 라이브 확대, 화면 고정 키(기본 Space)로 화면을 멈추고 툴바 조작
- **필기**: 펜 · 형광펜 · 지우개 · 텍스트, 선/사각형/타원/화살표 도형, 화이트보드/칠판 바탕
- **포인터 강조**: 포인터 후광, 클릭 파문
- **저장**: 화면 복사, 화면 저장(`사진\OniScreen`), MP4 녹화(`동영상\OniScreen`, 영상만)
- 트레이 아이콘, 다중 모니터, 전역 단축키 변경(설정)

## 기본 단축키

| 단축키 | 동작 |
| --- | --- |
| Ctrl+Alt+Z | 확대 켜기/끄기 |
| Ctrl+1 | 확대 (+) |
| Ctrl+2 / 3 / 4 / 5 | 펜 / 형광펜 / 지우개 / 텍스트 |
| Ctrl+6 | 포인터 강조 |
| Ctrl+7 | 모두 지우기 |
| Ctrl+0 | 화이트보드/칠판 |
| Ctrl+Alt+C / S | 화면 복사 / 저장 |
| Ctrl+Alt+R | 녹화 |
| Ctrl+Alt+Q | 종료 |

그리는 동안: 드래그 + Shift 선 · Ctrl 사각형 · Tab 타원 · Ctrl+Shift 화살표 · Ctrl+휠 펜 굵기 · R G B O Y K W 색상.

## 설치

[Releases](../../releases)에서 `OniScreen_x.y.z_x64-setup.exe`를 받아 실행합니다. Windows 10/11 x64, WebView2 런타임이 필요합니다(Windows 11 기본 포함).

## 개발

Rust(MSVC 툴체인), Node.js가 필요합니다.

```powershell
npm ci
npm run dev      # 개발 실행
npm test         # cargo test
npm run build    # target/release/bundle/nsis 에 설치 파일 생성
```

아이콘은 `scripts/make-icon.ps1`로 `icon-source.png`를 그린 뒤 `npm run icons`로 각 크기를 만듭니다.

`v*` 태그를 푸시하면 GitHub Actions(`.github/workflows/release.yml`)가 빌드해 릴리스에 설치 파일을 올립니다.

## 구성

- `src-tauri/` — 앱 본체(Rust): 확대(Magnification API), 녹화(Media Foundation), 창/단축키
- `ui/` — 툴바 · 오버레이 · 설정 화면
- `crates/screen-probe/`, `docs/`, `reports/`, `validation/` — 초기 검증용 Screen Probe 기록([docs/SCREEN-PROBE.md](docs/SCREEN-PROBE.md)), 현재 빌드 대상 아님
