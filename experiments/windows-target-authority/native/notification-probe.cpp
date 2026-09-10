// Isolated native notification observation; never a product adapter or admission test.
#include <windows.h>
#include <UIAutomation.h>
#include <winrt/base.h>
#include <array>
#include <atomic>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>

namespace {
constexpr wchar_t kClass[] = L"LensNotificationFixtureV1";
std::atomic<HWND> window{nullptr};
// Only the owning UI thread may use this handle for mutations/destruction.
thread_local HWND ownedWindow = nullptr;
enum class Command { None, FirstChange, SecondChange };
std::atomic<Command> pendingCommand{Command::None};
std::atomic<bool> stopRequested{false};
HANDLE ready = nullptr;
HANDLE changed = nullptr;
HANDLE requested = nullptr;
std::atomic<DWORD> fixtureError{ERROR_SUCCESS};
ULONGLONG started = 0;
LRESULT CALLBACK WindowProc(HWND target, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_CLOSE) {
        // Cancellation is observable, but destruction is deferred to the owning
        // command loop after the MTA reader has released its retained root.
        fixtureError = ERROR_CANCELLED;
        return 0;
    }
    if (message == WM_NCDESTROY && target == ownedWindow) {
        ownedWindow = nullptr;
        window = nullptr;
    }
    return DefWindowProcW(target, message, wparam, lparam);
}
DWORD WINAPI FixtureThread(void*) {
    WNDCLASSW type{};
    type.lpfnWndProc = WindowProc;
    type.hInstance = GetModuleHandleW(nullptr);
    type.lpszClassName = kClass;
    if (!RegisterClassW(&type)) fixtureError = GetLastError();
    else {
        ownedWindow = CreateWindowExW(WS_EX_NOACTIVATE, kClass, L"Lens notification initial",
            WS_OVERLAPPEDWINDOW, 100, 100, 480, 320, nullptr, nullptr, type.hInstance, nullptr);
        window = ownedWindow;
        if (!ownedWindow) fixtureError = GetLastError();
        else ShowWindow(ownedWindow, SW_SHOWNOACTIVATE);
    }
    SetEvent(ready);
    while (ownedWindow && !stopRequested.load()) {
        const DWORD wait = MsgWaitForMultipleObjects(1, &requested, FALSE, 2000, QS_ALLINPUT);
        if (wait == WAIT_OBJECT_0) {
            if (stopRequested.load()) break;
            const Command command = pendingCommand.exchange(Command::None);
            if (command == Command::None) { fixtureError = ERROR_INVALID_DATA; SetEvent(changed); continue; }
            const HWND target = ownedWindow;
            if (!SetWindowTextW(target, command == Command::SecondChange ? L"Lens notification second" : L"Lens notification first")) {
                fixtureError = GetLastError();
            } else if (ownedWindow == target) {
                NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, target, OBJID_WINDOW, CHILDID_SELF);
            } else {
                fixtureError = ERROR_INVALID_WINDOW_HANDLE;
            }
            SetEvent(changed);
        } else if (wait == WAIT_OBJECT_0 + 1) {
            MSG message{};
            for (unsigned count = 0; count < 64 && PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE); ++count) {
                if (message.message == WM_QUIT) { fixtureError = ERROR_CANCELLED; break; }
                TranslateMessage(&message); DispatchMessageW(&message);
            }
        } else if (wait != WAIT_TIMEOUT) {
            fixtureError = GetLastError(); Sleep(10);
        }
    }
    // Destroy only the still-live UI-thread-owned object, never a published/cached HWND.
    if (ownedWindow && !DestroyWindow(ownedWindow)) fixtureError = GetLastError();
    UnregisterClassW(kClass, type.hInstance);
    return 0;
}
struct Event { ULONGLONG elapsed; unsigned phase; };
class Sink final : public IUIAutomationPropertyChangedEventHandler {
    std::atomic<ULONG> references{1};
public:
    std::mutex mutex;
    std::array<Event, 32> events{};
    unsigned count = 0, phase = 0;
    bool overflow = false;
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** value) override {
        if (!value) return E_POINTER;
        *value = nullptr;
        if (id == __uuidof(IUnknown) || id == __uuidof(IUIAutomationPropertyChangedEventHandler)) {
            *value = static_cast<IUIAutomationPropertyChangedEventHandler*>(this);
            AddRef(); return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++references; }
    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG remaining = --references;
        if (!remaining) delete this;
        return remaining;
    }
    HRESULT STDMETHODCALLTYPE HandlePropertyChangedEvent(IUIAutomationElement*, PROPERTYID property, VARIANT) override {
        if (property != UIA_NamePropertyId) return E_INVALIDARG;
        const std::lock_guard<std::mutex> lock(mutex);
        if (count == events.size()) overflow = true;
        else events[count++] = {GetTickCount64() - started, phase};
        return S_OK;
    }
};
void Barrier(const char* name) {
    if (std::printf("{\"version\":1,\"scope\":\"notification-barrier\",\"barrier\":\"%s\"}\n", name) < 0 ||
        std::fflush(stdout) != 0) winrt::throw_hresult(E_FAIL);
    char command[16]{};
    if (!std::fgets(command, sizeof(command), stdin) || std::strcmp(command, "continue\n")) winrt::throw_hresult(E_ABORT);
}
std::string JsonString(BSTR text) {
    const UINT length = text ? SysStringLen(text) : 0;
    if (length > 1024) winrt::throw_hresult(E_BOUNDS);
    std::string result = "\"";
    // Escape every UTF-16 code unit: bounded, deterministic, no locale conversion.
    for (UINT i = 0; i < length; ++i) {
        char escape[7]{};
        std::snprintf(escape, sizeof(escape), "\\u%04x", static_cast<unsigned>(text[i]));
        result += escape;
    }
    return result + '"';
}
std::string Property(IUIAutomationElement* element, bool provider) {
    BSTR value = nullptr;
    const HRESULT result = provider ? element->get_CurrentProviderDescription(&value) : element->get_CurrentFrameworkId(&value);
    struct Free { BSTR value; ~Free() { SysFreeString(value); } } free{value};
    winrt::check_hresult(result);
    return JsonString(value);
}
void Change(bool second) {
    ResetEvent(changed);
    Command expected = Command::None;
    if (stopRequested.load() || !pendingCommand.compare_exchange_strong(expected, second ? Command::SecondChange : Command::FirstChange)) {
        winrt::throw_hresult(E_ABORT);
    }
    if (!SetEvent(requested)) winrt::throw_last_error();
    if (WaitForSingleObject(changed, 2000) != WAIT_OBJECT_0) winrt::throw_hresult(HRESULT_FROM_WIN32(WAIT_TIMEOUT));
    const DWORD error = fixtureError.load();
    if (error) winrt::throw_hresult(HRESULT_FROM_WIN32(error));
}
}

int main(int argc, char** argv) {
    if (argc != 2 || (std::strcmp(argv[1], "queued") && std::strcmp(argv[1], "unsubscribe-race"))) return 2;
    started = GetTickCount64();
    HRESULT result = S_OK;
    const char* stage = "fixture";
    std::string provider = "null", framework = "null", events = "[]";
    bool registered = false, initialized = false;
    unsigned queuedBeforeFence = 0;
    HANDLE thread = nullptr;
    winrt::com_ptr<IUIAutomation> automation;
    winrt::com_ptr<IUIAutomationElement> root;
    winrt::com_ptr<Sink> sink;
    try {
        ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        changed = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        requested = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (!ready || !changed || !requested) winrt::throw_last_error();
        thread = CreateThread(nullptr, 0, FixtureThread, nullptr, 0, nullptr);
        if (!thread) winrt::throw_last_error();
        if (WaitForSingleObject(ready, 2000) != WAIT_OBJECT_0) winrt::throw_hresult(HRESULT_FROM_WIN32(WAIT_TIMEOUT));
        if (fixtureError || !window) winrt::throw_hresult(E_FAIL);
        stage = "mta";
        winrt::init_apartment(winrt::apartment_type::multi_threaded); initialized = true;
        winrt::check_hresult(CoCreateInstance(__uuidof(CUIAutomation), nullptr, CLSCTX_INPROC_SERVER,
            __uuidof(IUIAutomation), automation.put_void()));
        stage = "root";
        winrt::check_hresult(automation->ElementFromHandle(window, root.put()));
        stage = "provider";
        provider = Property(root.get(), true); framework = Property(root.get(), false);
        sink.attach(new Sink());
        PROPERTYID property = UIA_NamePropertyId;
        stage = "register";
        winrt::check_hresult(automation->AddPropertyChangedEventHandlerNativeArray(root.get(), TreeScope_Element,
            nullptr, sink.get(), &property, 1));
        registered = true;
        Barrier("registered");
        stage = "change";
        Change(false);
        Barrier("change-requested");
        // queued mode permits a bounded delivery window; race mode removes immediately.
        if (!std::strcmp(argv[1], "queued")) Sleep(250);
        {
            const std::lock_guard<std::mutex> lock(sink->mutex);
            queuedBeforeFence = sink->count;
            sink->phase = 1;
        }
        stage = "unsubscribe";
        winrt::check_hresult(automation->RemovePropertyChangedEventHandler(root.get(), sink.get()));
        registered = false;
        { const std::lock_guard<std::mutex> lock(sink->mutex); sink->phase = 2; }
        Barrier("unsubscribed");
        stage = "post-unsubscribe-change";
        Change(true);
        Sleep(250);
        Barrier("settled");
        {
            const std::lock_guard<std::mutex> lock(sink->mutex);
            if (sink->overflow) winrt::throw_hresult(E_BOUNDS);
            events = "[";
            for (unsigned i = 0; i < sink->count; ++i) {
                if (i) events += ',';
                events += "{\"sequence\":" + std::to_string(i) + ",\"elapsed_ms\":" + std::to_string(sink->events[i].elapsed) +
                    ",\"phase\":" + std::to_string(sink->events[i].phase) + '}';
            }
            events += ']';
        }
        stage = "observed";
    } catch (const winrt::hresult_error& error) { result = error.code(); }
      catch (...) { result = E_FAIL; }
    if (registered) {
        const HRESULT removed = automation->RemovePropertyChangedEventHandler(root.get(), sink.get());
        if (FAILED(removed)) { result = removed; stage = "cleanup"; }
    }
    // UIA keeps its own COM references for late deliveries; never forcibly delete the sink.
    sink = nullptr; root = nullptr; automation = nullptr;
    if (initialized) winrt::uninit_apartment();
    stopRequested = true;
    if (requested && !SetEvent(requested)) { result = E_FAIL; stage = "cleanup"; }
    const bool joined = !thread || WaitForSingleObject(thread, 2000) == WAIT_OBJECT_0;
    if (!joined) { result = E_FAIL; stage = "cleanup"; }
    if (joined && fixtureError.load() != ERROR_SUCCESS && SUCCEEDED(result)) {
        result = HRESULT_FROM_WIN32(fixtureError.load()); stage = "cleanup";
    }
    if (thread) CloseHandle(thread);
    // A stalled UI thread may still signal these. On failure, process exit owns
    // reclamation; never close shared handles until the thread is joined.
    if (joined && ready) CloseHandle(ready);
    if (joined && changed) CloseHandle(changed);
    if (joined && requested) CloseHandle(requested);
    const int written = std::printf("{\"version\":1,\"scope\":\"native-notification-observation\",\"scenario\":\"%s\","
        "\"stage\":\"%s\",\"hresult\":%ld,\"pid\":%lu,\"provider_description\":%s,\"framework_id\":%s,"
        "\"queued_before_fence\":%u,\"events\":%s,\"product_admission_granted\":false}\n",
        argv[1], stage, static_cast<long>(result), GetCurrentProcessId(), provider.c_str(), framework.c_str(), queuedBeforeFence, events.c_str());
    return FAILED(result) || written < 0 || std::fflush(stdout) != 0 ? 1 : 0;
}
