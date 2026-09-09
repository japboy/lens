// Controlled real UIA property callback, not a sleeping client substitute.
#include <windows.h>
#include <UIAutomation.h>
#include <atomic>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <thread>

namespace {
enum class State { Ready, Armed, Entered, Released, Stopped };
std::atomic<State> state{State::Ready};
std::atomic<bool> failed{false};
std::atomic<unsigned> activeCalls{0};
HANDLE gate = nullptr;
HANDLE stopRequested = nullptr;
HWND window = nullptr;
std::mutex windowLifetime;
std::mutex output;
unsigned sequence = 0;
void Emit(const char* event, HRESULT hr = S_OK) {
    std::lock_guard<std::mutex> lock(output);
    if (std::printf("{\"version\":1,\"event\":\"%s\",\"seq\":%u,\"pid\":%lu,\"hresult\":%ld}\n",
        event, ++sequence, GetCurrentProcessId(), hr) < 0 || std::fflush(stdout) != 0) failed = true;
}
class Provider final : public IRawElementProviderSimple {
    std::atomic<ULONG> refs{1};
public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** value) override {
        if (!value) return E_POINTER;
        *value = nullptr;
        if (id == __uuidof(IUnknown) || id == __uuidof(IRawElementProviderSimple)) {
            *value = static_cast<IRawElementProviderSimple*>(this); AddRef(); return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
    ULONG STDMETHODCALLTYPE Release() override { const ULONG n = --refs; if (!n) delete this; return n; }
    HRESULT STDMETHODCALLTYPE get_ProviderOptions(ProviderOptions* value) override {
        if (!value) return E_POINTER; *value = ProviderOptions_ServerSideProvider; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetPatternProvider(PATTERNID, IUnknown** value) override {
        if (!value) return E_POINTER; *value = nullptr; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE get_HostRawElementProvider(IRawElementProviderSimple** value) override {
        if (!value) return E_POINTER;
        *value = nullptr;
        std::lock_guard<std::mutex> lock(windowLifetime);
        if (!window || state == State::Stopped) return UIA_E_ELEMENTNOTAVAILABLE;
        return UiaHostProviderFromHwnd(window, value);
    }
    HRESULT STDMETHODCALLTYPE GetPropertyValue(PROPERTYID property, VARIANT* value) override {
        if (!value) return E_POINTER;
        VariantInit(value);
        if (state == State::Stopped) return UIA_E_ELEMENTNOTAVAILABLE;
        if (property == UIA_ControlTypePropertyId) {
            value->vt = VT_I4; value->lVal = UIA_CustomControlTypeId; return S_OK;
        }
        if (property != UIA_HelpTextPropertyId) return S_OK;
        State expected = State::Armed;
        {
            std::lock_guard<std::mutex> lock(windowLifetime);
            if (!window || state == State::Stopped) return UIA_E_ELEMENTNOTAVAILABLE;
            if (!state.compare_exchange_strong(expected, State::Entered)) return S_OK;
            ++activeCalls;
        }
        struct ActiveCall { ~ActiveCall() { --activeCalls; } } active;
        Emit("provider_entered");
        // External supervisor deadline is shorter; this is only a cooperative failsafe.
        const DWORD wait = WaitForSingleObject(gate, 60000);
        HRESULT hr = wait == WAIT_OBJECT_0 ? S_OK : HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        if (state == State::Stopped) hr = UIA_E_ELEMENTNOTAVAILABLE;
        if (SUCCEEDED(hr)) {
            value->vt = VT_BSTR; value->bstrVal = SysAllocString(L"LENS_REAL_PROVIDER_RETURN_91A2");
            if (!value->bstrVal) hr = E_OUTOFMEMORY;
        }
        Emit("provider_returned", hr);
        return hr;
    }
};
Provider* provider = nullptr;
LRESULT CALLBACK Procedure(HWND hwnd, UINT message, WPARAM w, LPARAM l) {
    if (message == WM_GETOBJECT && provider) return UiaReturnRawElementProvider(hwnd, w, l, provider);
    if (message == WM_CLOSE) { SetEvent(stopRequested); return 0; }
    if (message == WM_DESTROY) {
        { std::lock_guard<std::mutex> lock(windowLifetime); if (window == hwnd) window = nullptr; }
        SetEvent(stopRequested); SetEvent(gate);
        UiaReturnRawElementProvider(hwnd, 0, 0, nullptr); PostQuitMessage(0); return 0;
    }
    return DefWindowProcW(hwnd, message, w, l);
}
void Control() {
    char command[64]{};
    unsigned length = 0;
    unsigned commands = 0;
    const HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    while (WaitForSingleObject(stopRequested, 10) == WAIT_TIMEOUT) {
        DWORD available = 0;
        if (!PeekNamedPipe(input, nullptr, 0, nullptr, &available, nullptr)) break;
        if (!available) continue;
        char byte = 0; DWORD count = 0;
        if (!ReadFile(input, &byte, 1, &count, nullptr) || count != 1) break;
        if (byte != '\n') {
            if (length >= sizeof(command) - 1 || byte == '\0') break;
            command[length++] = byte; continue;
        }
        if (++commands > 8) break;
        if (length && command[length - 1] == '\r') --length;
        command[length] = 0; length = 0;
        State expected = State::Ready;
        if (!std::strcmp(command, "arm") && state.compare_exchange_strong(expected, State::Armed)) {
            Emit("armed"); continue;
        }
        expected = State::Entered;
        if (!std::strcmp(command, "release") && state.compare_exchange_strong(expected, State::Released)) {
            Emit("released"); SetEvent(gate); continue;
        }
        if (!std::strcmp(command, "stop")) {
            state = State::Stopped; Emit("stopping"); SetEvent(gate); SetEvent(stopRequested); return;
        }
        break;
    }
    if (WaitForSingleObject(stopRequested, 0) == WAIT_OBJECT_0) return;
    failed = true; state = State::Stopped; Emit("protocol_failed", E_INVALIDARG);
    SetEvent(gate); SetEvent(stopRequested);
}
}
int main() {
    if (GetFileType(GetStdHandle(STD_INPUT_HANDLE)) != FILE_TYPE_PIPE) return 2;
    if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) return 2;
    gate = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    stopRequested = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!gate || !stopRequested) return 2;
    WNDCLASSW cls{}; cls.lpfnWndProc = Procedure; cls.hInstance = GetModuleHandleW(nullptr);
    cls.lpszClassName = L"LensProviderStallFixtureV1";
    if (!RegisterClassW(&cls)) return 2;
    window = CreateWindowExW(0, cls.lpszClassName, L"Lens Controlled Provider Stall", WS_OVERLAPPEDWINDOW,
        20, 20, 320, 160, nullptr, nullptr, cls.hInstance, nullptr);
    if (!window) return 2;
    provider = new Provider(); ShowWindow(window, SW_SHOW);
    Emit("ready");
    std::thread control(Control);
    bool quit = false;
    while (!quit) {
        const DWORD ready = MsgWaitForMultipleObjects(1, &stopRequested, FALSE, INFINITE, QS_ALLINPUT);
        if (ready == WAIT_OBJECT_0) break;
        if (ready != WAIT_OBJECT_0 + 1) { failed = true; break; }
        MSG message{};
        while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
            if (message.message == WM_QUIT) { quit = true; break; }
            TranslateMessage(&message); DispatchMessageW(&message);
        }
    }
    // Supervisor must stop through the pipe. Unexpected UI close is an unsuccessful run.
    if (state != State::Stopped) { failed = true; state = State::Stopped; }
    SetEvent(gate); SetEvent(stopRequested);
    HWND owned = nullptr;
    { std::lock_guard<std::mutex> lock(windowLifetime); owned = window; window = nullptr; }
    // Only the creating UI thread destroys this still-owned window. Host-provider
    // lookup cannot overlap the transition to a closed lifetime.
    if (owned && !DestroyWindow(owned)) failed = true;
    control.join();
    const ULONGLONG until = GetTickCount64() + 5000;
    while (activeCalls.load() && GetTickCount64() < until) Sleep(10);
    // Never close an event handle while a provider may still be waiting on it.
    // Abnormal in-process teardown is failure; supervisor still owns termination.
    if (activeCalls.load()) ExitProcess(2);
    provider->Release(); CloseHandle(gate); CloseHandle(stopRequested); CoUninitialize();
    Emit("closed"); return failed ? 2 : 0;
}
