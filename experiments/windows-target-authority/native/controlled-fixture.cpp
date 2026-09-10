// Experimental controlled oracle only. Never compiled into the product.
#include <windows.h>
#include <cstdio>
#include <cstring>
#include <cstdint>
#include <initializer_list>

namespace {
constexpr wchar_t kClass[] = L"LensControlledAuthorityFixtureV1";
enum class State { Empty, A, B, Stopped };
State state = State::Empty;
HWND owned = nullptr;
unsigned step = 0;
ULONGLONG started = 0;
bool output_failed = false;

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
    }
    if (message == WM_CLOSE) {
        // An interactive close ends this run; it cannot silently replace the oracle.
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
    if (message == WM_PAINT) {
        PAINTSTRUCT paint{};
        HDC dc = BeginPaint(window, &paint);
        RECT rect{};
        GetClientRect(window, &rect);
        HBRUSH brush = CreateSolidBrush(state == State::A ? RGB(30, 80, 220) : RGB(40, 210, 70));
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
    owned = CreateWindowExW(WS_EX_NOACTIVATE, kClass,
        next == State::A ? L"Lens fixture A" : L"Lens fixture B",
        WS_OVERLAPPEDWINDOW, 100, 100, 480, 320, nullptr, nullptr,
        GetModuleHandleW(nullptr), nullptr);
    if (owned == nullptr) { Observe("create_failed", GetLastError()); return false; }
    ShowWindow(owned, SW_SHOWNOACTIVATE);
    UpdateWindow(owned);
    Observe("created"); // Not proof that compositor presentation has completed.
    return true;
}
bool Command(const char* command) {
    if (std::strcmp(command, "stop") == 0) { state = State::Stopped; return true; }
    if (std::strcmp(command, "create-a") == 0 && state == State::Empty) return Create(State::A);
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
        if (GetTickCount64() - started >= 30000) { Observe("deadline"); result = 3; break; }
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
            if (++commands > 16 || !Command(line)) { result = 2; break; }
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
