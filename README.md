# OniScreen

강의·발표용 Windows 화면 확대 · 필기 · 포인터 강조 도구입니다. (Tauri 2 + Rust, 이전 이름: Oni Zoom)

## 기능

- **확대**: 1~6배 라이브 확대, 라이브 줌(마우스 추적) / 줌(위치 고정, 가운데 버튼 드래그) 전환, 라이브 줌에서만 Space로 추적 잠금/해제
- **필기**: 펜 · 형광펜 · 지우개 · 텍스트, 선/사각형/타원/화살표 도형, 화이트보드/칠판 바탕
- **포인터 강조**: 포인터 후광, 클릭 파문
- **화면 복사**: 현재 화면을 클립보드에 복사
- 트레이 아이콘, 다중 모니터, 전역 단축키 변경(설정)

## 기본 단축키

| 단축키 | 동작 |
| --- | --- |
| Ctrl+Alt+Z | 확대 켜기/끄기 |
| Ctrl+1 | 일반 줌 시작 / 확대 (+) |
| Ctrl+Shift+1 | 일반 줌 ↔ 라이브 줌 |
| Ctrl+2 / 3 / 4 / 5 | 펜 / 형광펜 / 지우개 / 텍스트 |
| Ctrl+6 | 포인터 강조 |
| Ctrl+7 | 모두 지우기 |
| Ctrl+0 | 화이트보드/칠판 |
| Ctrl+Alt+C | 화면 복사 |
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

- `src-tauri/` — 앱 본체(Rust): 확대(Magnification API), 창/단축키
- `ui/` — 툴바 · 오버레이 · 설정 화면
- `crates/screen-probe/`, `docs/`, `reports/`, `validation/` — 초기 검증용 Screen Probe 기록([docs/SCREEN-PROBE.md](docs/SCREEN-PROBE.md)), 현재 빌드 대상 아님

### 업데이트
설정 → 업데이트 확인에서 GitHub 최신 정식 릴리스와 현재 버전을 비교합니다. 새 버전이 있으면 **설치 파일 받기**로 기본 브라우저에서 Windows 설치 파일을 다운로드하고 실행합니다. 앱 내부 자동 설치 방식은 아닙니다.
배포 시 package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json 버전을 동일하게 올린 뒤 해당 버전의 v 태그를 푸시합니다. 기존 release 워크플로가 `*-setup.exe`를 GitHub Releases에 게시합니다. 공개 릴리스가 없거나 네트워크 오류가 나면 설정창에 이유를 표시합니다.

### 화면 공유
Google Meet 등에서는 **전체 화면 공유**를 선택하세요. v0.2.1부터 확대·필기·포인터 표시가 공유 화면에 포함됩니다. 내부 확대는 Windows Magnification API의 창 필터로 자기 창을 제외하며, 메뉴바와 설정창은 공유 화면에서 숨깁니다. 창 또는 탭만 공유하면 별도 오버레이는 포함되지 않을 수 있습니다.
