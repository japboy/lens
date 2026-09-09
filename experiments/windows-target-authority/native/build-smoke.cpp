// Compile/link experiment only. The metadata entry point never invokes source APIs.
#include <windows.h>
#include <roapi.h>
#include <UIAutomation.h>
#include <windows.graphics.capture.h>
#include <windows.graphics.capture.interop.h>
#include <wrl/client.h>
#include <wrl/wrappers/corewrappers.h>
#include <cstdio>
#include <cstring>

#if !defined(_MSC_VER) || !defined(_M_X64)
#error This experiment requires MSVC targeting x64.
#endif

// /INCLUDE retains this function even though the executable never calls it.
// A future native harness must own MTA initialization and window authority before
// using these APIs. This function is NOT an operational acquisition implementation.
extern "C" HRESULT LensSdkLinkSmoke(HWND window) {
    using Microsoft::WRL::ComPtr;
    using Microsoft::WRL::Wrappers::HStringReference;
    ComPtr<IUIAutomation> automation;
    HRESULT result = CoCreateInstance(__uuidof(CUIAutomation), nullptr,
        CLSCTX_INPROC_SERVER, IID_PPV_ARGS(automation.GetAddressOf()));
    if (FAILED(result)) return result;
    ComPtr<IUIAutomationElement> element;
    result = automation->ElementFromHandle(window, element.GetAddressOf());
    if (FAILED(result)) return result;
    ComPtr<IGraphicsCaptureItemInterop> factory;
    result = RoGetActivationFactory(
        HStringReference(L"Windows.Graphics.Capture.GraphicsCaptureItem").Get(),
        IID_PPV_ARGS(factory.GetAddressOf()));
    if (FAILED(result)) return result;
    ComPtr<ABI::Windows::Graphics::Capture::IGraphicsCaptureItem> item;
    return factory->CreateForWindow(window, IID_PPV_ARGS(item.GetAddressOf()));
}

int main(int argc, char** argv) {
    if (argc != 2 || std::strcmp(argv[1], "--metadata") != 0) {
        std::fputs("Only --metadata is supported; no capture or UIA runtime test exists.\n", stderr);
        return 2;
    }
    std::printf("{\"version\":1,\"scope\":\"sdk-build-smoke-only\","
        "\"architecture\":\"x64\",\"msvc_full_version\":%d,"
        "\"pointer_bytes\":%zu,\"native_acquisition_executed\":false,"
        "\"product_admission_granted\":false}\n", _MSC_FULL_VER, sizeof(void*));
    return 0;
}
