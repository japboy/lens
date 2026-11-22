// Experimental controlled oracle only. Never compiled into the product.
#include <windows.h>
#include <cstdio>
#include <cstring>
#include <cstdint>
#include <initializer_list>

#include <string>

namespace {
constexpr wchar_t kClass[] = L"LensControlledAuthorityFixtureV1";
// Allow the 60 s selection, two 6 s reads and commits with 25 s pauses,
// and the final held read (25 s), plus time for cleanup.
constexpr ULONGLONG kReservationMs = 180'000;
constexpr ULONGLONG kShortReservationMs = 350;
constexpr ULONGLONG kOwnerLifetimeMs = kReservationMs + 30'000;
enum class State { Empty, A, B, Stopped };
State state = State::Empty;
HWND owned = nullptr;
unsigned step = 0;
ULONGLONG started = 0;
bool output_failed = false;
HWND label = nullptr;
HBITMAP marker_bitmap = nullptr;
bool repainted = false;

COLORREF FixtureColour() {
    if (repainted) return RGB(170, 115, 25);
    return state == State::A ? RGB(30, 80, 220) : RGB(40, 210, 70);
}
bool PaintMarkerBitmap() {
    HDC target = GetDC(owned);
    HDC dc = target ? CreateCompatibleDC(target) : nullptr;
    if (target) ReleaseDC(owned, target);
    if (!dc) return false;
    const auto old = SelectObject(dc, marker_bitmap);
    RECT rect{0, 0, 64, 40};
    HBRUSH brush = CreateSolidBrush(FixtureColour());
    bool painted = brush && FillRect(dc, &rect, brush);
    if (brush) DeleteObject(brush);
    // Integer, non-antialiased fiducials. The validation oracle independently
    // specifies these pixels; neither bounds nor success come from UIA/crop code.
    const struct { RECT rect; COLORREF colour; } fiducials[] = {
        {{0, 0, 64, 1}, RGB(245, 245, 245)},
        {{0, 39, 64, 40}, RGB(245, 245, 245)},
        {{0, 0, 1, 40}, RGB(245, 245, 245)},
        {{63, 0, 64, 40}, RGB(245, 245, 245)},
        {{1, 1, 9, 7}, RGB(231, 31, 39)},
        {{53, 1, 63, 5}, RGB(17, 211, 224)},
        {{1, 29, 6, 39}, RGB(247, 213, 19)},
        {{47, 31, 63, 39}, RGB(145, 37, 211)},
        {{28, 9, 31, 28}, RGB(9, 12, 15)},
    };
    for (const auto& fiducial : fiducials) {
        brush = CreateSolidBrush(fiducial.colour);
        if (!brush) { painted = false; continue; }
        if (!FillRect(dc, &fiducial.rect, brush)) painted = false;
        DeleteObject(brush);
    }
    SelectObject(dc, old);
    DeleteDC(dc);
    return painted;
}

std::string lease;
ULONGLONG leaseDeadline = 0;
unsigned generation = 0;
bool LeaseLive() { return !lease.empty() && GetTickCount64() < leaseDeadline; }
void LeaseRecord(const char* event, const char* nonce, bool granted) {
    const auto now = GetTickCount64();
    std::printf("{\"event\":\"%s\",\"nonce\":\"%s\",\"granted\":%s,"
        "\"generation\":%u,\"handle\":\"0x%llx\",\"pid\":%lu,\"remaining_ms\":%llu}\n",
        event, nonce, granted ? "true" : "false", generation,
        static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(owned)), GetCurrentProcessId(),
        !lease.empty() && now < leaseDeadline ? leaseDeadline - now : 0);
    std::fflush(stdout);
}

const char* Marker() {
    return state == State::A ? "A" : state == State::B ? "B" : "none";
}
void Observe(const char* event, DWORD error = ERROR_SUCCESS) {
    const int written = std::printf("{\"version\":1,\"scope\":\"controlled-fixture-observation\","
        "\"step\":%u,\"elapsed_ms\":%llu,\"event\":\"%s\",\"marker\":\"%s\","
        "\"handle\":\"0x%llx\",\"pid\":%lu,\"win32_error\":%lu,"
        "\"product_admission_granted\":false}\n", step++,
        GetTickCount64() - started, event, Marker(),
        static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(owned)),
        GetCurrentProcessId(), error);
    if (written < 0 || std::fflush(stdout) == EOF) output_failed = true;
}
LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_NCDESTROY && window == owned) {
        owned = nullptr;
        if (marker_bitmap) { DeleteObject(marker_bitmap); marker_bitmap = nullptr; }
    }
    if (message == WM_CLOSE) {
        // An interactive close ends this run; it cannot silently replace the oracle.
        if (LeaseLive()) { Observe("owner-refused-wm-close"); return 0; }
        state = State::Stopped;
        return 0;
    }
    if (message == WM_DPICHANGED) {
        const auto* rect = reinterpret_cast<const RECT*>(lparam);
        SetWindowPos(window, nullptr, rect->left, rect->top,
            rect->right - rect->left, rect->bottom - rect->top,
            SWP_NOZORDER | SWP_NOACTIVATE);
        return 0;
    }
    if (message == WM_CTLCOLORSTATIC && reinterpret_cast<HWND>(lparam) == label) {
        const auto dc = reinterpret_cast<HDC>(wparam);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, RGB(255, 255, 255));
        SetDCBrushColor(dc, RGB(190, 20, 146));
        return reinterpret_cast<LRESULT>(GetStockObject(DC_BRUSH));
    }
    if (message == WM_PAINT) {
        PAINTSTRUCT paint{};
        HDC dc = BeginPaint(window, &paint);
        RECT rect{};
        GetClientRect(window, &rect);
        HBRUSH brush = CreateSolidBrush(RGB(22, 27, 34));
        if (brush != nullptr) { FillRect(dc, &rect, brush); DeleteObject(brush); }
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, RGB(255, 255, 255));
        const wchar_t* text = state == State::A ? L"Lens fixture A" : L"Lens fixture B";
        DrawTextW(dc, text, -1, &rect, DT_LEFT | DT_TOP | DT_SINGLELINE);
        EndPaint(window, &paint);
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}
bool Create(State next) {
    state = next;
    repainted = false;
    owned = CreateWindowExW(WS_EX_NOACTIVATE, kClass,
        next == State::A ? L"Lens fixture A" : L"Lens fixture B",
        WS_OVERLAPPEDWINDOW, 100, 100, 480, 320, nullptr, nullptr,
        GetModuleHandleW(nullptr), nullptr);
    if (owned == nullptr) { Observe("create_failed", GetLastError()); return false; }
    ++generation;

    label = CreateWindowExW(0, L"STATIC", L"Lens geometry marker",
        WS_CHILD | WS_VISIBLE | SS_LEFT, 34, 52, 170, 36, owned, nullptr,
        GetModuleHandleW(nullptr), nullptr);
    if (!label) { Observe("label_create_failed", GetLastError()); return false; }
    HWND image = CreateWindowExW(0, L"STATIC", nullptr, WS_CHILD | WS_VISIBLE | SS_BITMAP,
        245, 72, 64, 40, owned, nullptr, GetModuleHandleW(nullptr), nullptr);
    if (!image) { Observe("image_create_failed", GetLastError()); return false; }
    HDC target = GetDC(owned);
    marker_bitmap = target ? CreateCompatibleBitmap(target, 64, 40) : nullptr;
    if (target) ReleaseDC(owned, target);
    if (!marker_bitmap) {
        Observe("bitmap_create_failed", GetLastError());
        return false;
    }
    if (!PaintMarkerBitmap()) { Observe("bitmap_paint_failed", GetLastError()); return false; }
    SendMessageW(image, STM_SETIMAGE, IMAGE_BITMAP, reinterpret_cast<LPARAM>(marker_bitmap));
    ShowWindow(owned, SW_SHOWNOACTIVATE);
    UpdateWindow(owned);
    Observe("created"); // Not proof that compositor presentation has completed.
    return true;
}
bool Command(const char* command) {
    if (!LeaseLive()) lease.clear();
    const std::string input(command);
    for (const auto* request : {"reserve ", "reserve-short ", "validate ", "commit ", "release ", "refresh ", "destroy "}) {
        const std::string prefix(request);
        if (!input.starts_with(prefix)) continue;
        const std::string nonce = input.substr(prefix.size());
        if (nonce.size() != 32 || nonce.find_first_not_of("0123456789abcdef") != std::string::npos) return false;
        bool granted = false;
        const bool reserving = prefix == "reserve " || prefix == "reserve-short ";
        if (reserving && owned && lease.empty()) {
            lease = nonce;
            leaseDeadline = GetTickCount64() +
                (prefix == "reserve-short " ? kShortReservationMs : kReservationMs);
            granted = true;
        } else if (!reserving) {
            granted = owned && LeaseLive() && nonce == lease;
        }
        if (granted && prefix == "refresh ") {
            repainted = !repainted;
            granted = SetWindowTextW(label, repainted ? L"Lens geometry marker updated" : L"Lens geometry marker") &&
                PaintMarkerBitmap() &&
                RedrawWindow(owned, nullptr, nullptr, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW);
        }
        const auto verb = prefix.substr(0, prefix.size() - 1);
        LeaseRecord(verb.c_str(), nonce.c_str(), granted);
        if (granted && prefix == "release ") lease.clear();
        if (granted && prefix == "destroy ") {
            if (!DestroyWindow(owned)) { Observe("destroy_failed", GetLastError()); return false; }
            lease.clear();
            state = State::Empty;
            Observe("closed");
        }
        return true;
    }
    if (LeaseLive() && (input == "close-a" || input == "replace-b" || input == "stop")) {
        Observe("owner-refused-change");
        return true;
    }
    if (std::strcmp(command, "stop") == 0) { state = State::Stopped; return true; }
    if (std::strcmp(command, "create-a") == 0 && state == State::Empty) return Create(State::A);
    if (std::strcmp(command, "create-b") == 0 && state == State::Empty) return Create(State::B);
    if (std::strcmp(command, "repaint") == 0 && state == State::A) {
        repainted = true;
        RedrawWindow(owned, nullptr, nullptr, RDW_INVALIDATE | RDW_UPDATENOW);
        Observe("repainted");
        return true;
    }
    if (std::strcmp(command, "close-a") == 0 && state == State::A) {
        if (!DestroyWindow(owned)) { Observe("destroy_failed", GetLastError()); return false; }
        state = State::Empty;
        Observe("closed");
        return true;
    }
    if (std::strcmp(command, "replace-b") == 0 && state == State::A) {
        const auto previous = reinterpret_cast<uintptr_t>(owned);
        if (!DestroyWindow(owned)) { Observe("destroy_failed", GetLastError()); return false; }
        owned = nullptr;
        if (!Create(State::B)) return false;
        Observe(previous == reinterpret_cast<uintptr_t>(owned) ? "handle_reused" : "handle_not_reused");
        return true;
    }
    if (state == State::A || state == State::B) {
        for (const char* barrier : {"before-root", "before-capture", "before-probe", "before-commit"}) {
            if (std::strcmp(command, barrier) == 0) { Observe(barrier); return true; }
        }
    }
    Observe("invalid_command", ERROR_INVALID_DATA);
    return false;
}
} // namespace

int main() {
    started = GetTickCount64();
    // Must precede all window creation. A future build should prefer a manifest.
    if (!SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)) {
        Observe("dpi_setup_failed", GetLastError()); return 2;
    }
    HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    if (GetFileType(input) != FILE_TYPE_PIPE) { Observe("stdin_pipe_required"); return 2; }
    WNDCLASSW windowClass{};
    windowClass.lpfnWndProc = WindowProc;
    windowClass.hInstance = GetModuleHandleW(nullptr);
    windowClass.lpszClassName = kClass;
    if (!RegisterClassW(&windowClass)) { Observe("class_failed", GetLastError()); return 2; }
    Observe("ready");
    char line[64]{};
    size_t used = 0;
    unsigned commands = 0;
    int result = 0;
    while (state != State::Stopped) {
        if (output_failed) { result = 2; break; }
        if (GetTickCount64() - started >= kOwnerLifetimeMs) { Observe("deadline"); result = 3; break; }
        MSG message{};
        for (unsigned count = 0; count < 64 && PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE); ++count) {
            TranslateMessage(&message); DispatchMessageW(&message);
        }
        DWORD available = 0;
        if (!PeekNamedPipe(input, nullptr, 0, nullptr, &available, nullptr)) {
            Observe("input_closed", GetLastError()); result = 2; break;
        }
        if (available == 0) { Sleep(10); continue; }
        char value = 0;
        DWORD read = 0;
        if (!ReadFile(input, &value, 1, &read, nullptr) || read != 1) { result = 2; break; }
        if (value == '\n') {
            if (used != 0 && line[used - 1] == '\r') --used;
            line[used] = '\0';
            if (++commands > 40 || !Command(line)) { result = 2; break; }
            used = 0;
        } else if (value == '\0' || used == sizeof(line) - 1) {
            Observe("invalid_line", ERROR_INVALID_DATA); result = 2; break;
        } else { line[used++] = value; }
    }
    if (owned != nullptr) {
        if (!DestroyWindow(owned)) { Observe("cleanup_failed", GetLastError()); result = 2; }
        owned = nullptr;
    }
    state = State::Stopped;
    Observe("stopped");
    UnregisterClassW(kClass, GetModuleHandleW(nullptr));
    return output_failed ? 2 : result;
}
