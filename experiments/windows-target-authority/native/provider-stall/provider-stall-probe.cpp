// One windowless MTA client; the property call is made against an owned fixture PID.
#include <windows.h>
#include <UIAutomation.h>
#include <wrl/client.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>
#include <limits>
using Microsoft::WRL::ComPtr;
namespace {
DWORD expectedPid = 0;
HWND target = nullptr;
bool duplicate = false;
unsigned sequence = 0;
BOOL CALLBACK Find(HWND window, LPARAM) {
    DWORD pid = 0; GetWindowThreadProcessId(window, &pid);
    wchar_t cls[64]{}; GetClassNameW(window, cls, 64);
    if (pid == expectedPid && !std::wcscmp(cls, L"LensProviderStallFixtureV1")) {
        if (target) { duplicate = true; return FALSE; }
        target = window;
    }
    return TRUE;
}
bool Emit(const char* event, HRESULT hr, bool marker = false) {
    return std::printf("{\"version\":1,\"event\":\"%s\",\"seq\":%u,\"pid\":%lu,\"hresult\":%ld,\"marker\":%s}\n",
        event, ++sequence, GetCurrentProcessId(), hr, marker ? "true" : "false") >= 0 && std::fflush(stdout) == 0;
}
}
int main(int argc, char** argv) {
    if (argc != 2 || GetFileType(GetStdHandle(STD_INPUT_HANDLE)) != FILE_TYPE_PIPE) return 2;
    char* end = nullptr; const unsigned long long pid = std::strtoull(argv[1], &end, 10);
    if (!end || *end || !pid || pid > (std::numeric_limits<DWORD>::max)()) return 2;
    expectedPid = static_cast<DWORD>(pid);
    if (FAILED(CoInitializeEx(nullptr, COINIT_MULTITHREADED))) return 2;
    EnumWindows(Find, 0);
    ComPtr<IUIAutomation> automation; ComPtr<IUIAutomationElement> root;
    HRESULT hr = target && !duplicate ? CoCreateInstance(CLSID_CUIAutomation, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&automation)) : E_FAIL;
    if (SUCCEEDED(hr)) hr = automation->ElementFromHandle(target, &root);
    if (FAILED(hr)) { Emit("failed", hr); return 2; }
    if (!Emit("root_ready", S_OK)) return 2;
    char command[16]{};
    if (!std::fgets(command, sizeof(command), stdin) || std::strcmp(command, "go\n")) return 2;
    if (!Emit("call_started", S_OK)) return 2;
    VARIANT value; VariantInit(&value);
    hr = root->GetCurrentPropertyValueEx(UIA_HelpTextPropertyId, TRUE, &value);
    const bool marker = SUCCEEDED(hr) && value.vt == VT_BSTR && value.bstrVal &&
        !std::wcscmp(value.bstrVal, L"LENS_REAL_PROVIDER_RETURN_91A2");
    const bool emitted = Emit("call_returned", hr, marker);
    VariantClear(&value); root.Reset(); automation.Reset(); CoUninitialize();
    return emitted && SUCCEEDED(hr) && marker ? 0 : 2;
}
