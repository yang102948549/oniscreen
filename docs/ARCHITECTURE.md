# 확대·필기 0.2 구조

2026-10-13 기준. 이전 사고 엔진은 빌드 대상에서 제외하고 아래 구조로 교체했습니다.

## 프로세스 경계

1. `controller.rs`: 외부 실행 옵션을 분류합니다. 기본 실행은 supervisor만 시작합니다.
2. `supervisor.rs`: 비상키 준비 → Job 생성(KILL_ON_JOB_CLOSE) → 작업 프로세스 생성 → Job 등록 → GO 신호 → 시작/응답/세션 기한 감시. Job 등록 실패 시 작업 프로세스는 초기화 전에 종료합니다.
3. `presentation.rs`: 자체 창 안의 일반 Windows 메시지로 확대·필기를 조작합니다. 실제 입력을 가로채는 훅, 입력 주입, 시스템 커서 변경이 없습니다. 상태를 수정하는 Rust 대여가 끝난 뒤 창 API를 호출하여 재진입 시 중복 가변 참조를 피합니다.
4. `capture.rs`: 일회성 GDI 캡처를 별도 프로세스에서 처리합니다. 캡처도 Job과 시작 신호로 보호하며, 화면 데이터는 제한된 크기의 바이너리 파이프로 전달합니다. UI는 기다리지 않고 타이머에서 결과를 확인합니다.
5. `engine.rs`, `geometry.rs`: 운영체제와 독립적인 프레임 검증, 배율/좌표, 필기 상태, 메모리 렌더링입니다.

Windows는 마지막 Job 핸들이 닫힐 때 KILL_ON_JOB_CLOSE가 설정된 Job의 프로세스들을 종료합니다. [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects). 자동 검사에는 Rust 정리를 거치지 않는 감시 프로세스의 급작스러운 종료를 포함합니다.

## 확대와 입력

정지 확대는 캡처 작업의 픽셀을 메모리에서 확대하고 필기를 합성합니다. 라이브 확대는 감시받는 presentation 프로세스 안에서만 Magnifier 자식 컨트롤을 초기화합니다. 창 기반 변환만 쓰며 시스템 전체 확대 변환이나 커서 API는 호출하지 않습니다. 초기화/렌더링이 멈춰도 독립 감시는 계속 동작합니다.

라이브 필기층은 입력을 받지 않는 투명 자식 창입니다. Magnifier 자식 창도 입력을 비활성화하여 입력을 부모 창으로 모읍니다. 비활성 자식 창의 마우스 입력은 부모로 전달됩니다. [Microsoft Window Features](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features). Windows 10/11 호환 manifest를 포함하여 layered child window를 지원합니다.

입력은 원래 앱으로 변환하지 않습니다. 표시 중인 확대 화면에서 보기·필기만 하며 원래 앱 조작은 확대를 종료하고 수행합니다. 확대 호스트는 일반 z-order의 창으로 Alt+Tab을 막지 않습니다. Esc는 즉시 전체 확대를 해제합니다. Ctrl+0은 supervisor가 처리합니다.

라이브 필기층과 호스트는 Magnifier 자체의 제외 목록에만 들어갑니다. `SetWindowDisplayAffinity`로 회의 공유에서 숨기지 않습니다. 실제 Zoom/Meet 수신은 미검증입니다. [MagSetWindowFilterList](https://learn.microsoft.com/en-us/windows/win32/api/magnification/nf-magnification-magsetwindowfilterlist).

## 검사 경계

숨긴 UI 검사에서는 실제 캡처 및 Magnification 진입을 런타임 가드로 거부합니다. 전체 화면 배치·포커스 변경·전역 키 등록도 실행하지 않습니다. 합성 이미지의 확대/필기, 실제 창 프로시저, GDI 메모리 출력 및 투명 필기층을 확인합니다.

합성 시험은 실제 GPU/드라이버의 확대 처리, 화면 공유 수신, 물리 입력과 다중 DPI 조합을 검증하지 않습니다. 드라이버 또는 Windows 자체가 멈추는 상황까지 프로세스 격리로 방지한다고 주장하지 않습니다.
