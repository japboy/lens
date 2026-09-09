// Isolated experimental reader. Windows SDK compilation/execution remains unverified.
#include <windows.h>
#include <UIAutomation.h>
#include <d3d11.h>
#include <dxgi.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <charconv>
#include <cstdio>
#include <cstring>
#include <string>
#include <cstdint>
#include <cwchar>

using namespace winrt;
using namespace winrt::Windows::Graphics::Capture;
using namespace winrt::Windows::Graphics::DirectX;
using namespace winrt::Windows::Graphics::DirectX::Direct3D11;

namespace {
void Barrier(const char* name) {
    if (std::printf("{\"version\":1,\"scope\":\"acquisition-barrier\",\"barrier\":\"%s\"}\n", name) < 0 ||
        std::fflush(stdout) != 0) throw_hresult(E_FAIL);
    char line[16]{};
    if (!std::fgets(line, sizeof(line), stdin) || std::strcmp(line, "continue\n")) throw_hresult(E_ABORT);
}
std::string RuntimeId(IUIAutomationElement* element) {
    SAFEARRAY* array = nullptr;
    check_hresult(element->GetRuntimeId(&array));
    struct Cleanup { SAFEARRAY* value; ~Cleanup() { if (value) SafeArrayDestroy(value); } } cleanup{array};
    if (!array || SafeArrayGetDim(array) != 1) throw_hresult(E_UNEXPECTED);
    VARTYPE type = VT_EMPTY;
    check_hresult(SafeArrayGetVartype(array, &type));
    if (type != VT_I4) throw_hresult(E_UNEXPECTED);
    LONG lower = 0, upper = 0;
    check_hresult(SafeArrayGetLBound(array, 1, &lower));
    check_hresult(SafeArrayGetUBound(array, 1, &upper));
    const auto count = static_cast<int64_t>(upper) - lower + 1;
    if (count < 1 || count > 64) throw_hresult(E_BOUNDS);
    std::string result = "[";
    for (int64_t offset = 0; offset < count; ++offset) {
        LONG index = static_cast<LONG>(static_cast<int64_t>(lower) + offset), value = 0;
        check_hresult(SafeArrayGetElement(array, &index, &value));
        if (offset) result += ',';
        result += std::to_string(value);
    }
    return result + ']';
}
uint64_t Parse(const char* value, int base) {
    uint64_t result = 0;
    const size_t size = std::strlen(value);
    const auto parsed = std::from_chars(value, value + size, result, base);
    if (!size || parsed.ec != std::errc{} || parsed.ptr != value + size || !result) throw_hresult(E_INVALIDARG);
    return result;
}
}

int main(int argc, char** argv) {
    const char* stage = "input";
    std::string rootId = "null", probeId = "null";
    const char* equalJson = "null";
    ULONGLONG started = GetTickCount64();
    bool initialized = false;
    GraphicsCaptureSession session{nullptr};
    Direct3D11CaptureFramePool pool{nullptr};
    Direct3D11CaptureFrame frame{nullptr};
    HRESULT outcome = S_OK;
    unsigned blue = 0, green = 0, red = 0, alpha = 0;
    int width = 0, height = 0;
    bool sampled = false;
    try {
        if (argc != 3 || std::strlen(argv[1]) < 3 || std::strncmp(argv[1], "0x", 2)) throw_hresult(E_INVALIDARG);
        const uint64_t handle = Parse(argv[1] + 2, 16), expectedPid = Parse(argv[2], 10);
        if (handle > UINTPTR_MAX || expectedPid > MAXDWORD) throw_hresult(E_INVALIDARG);
        const HWND window = reinterpret_cast<HWND>(static_cast<uintptr_t>(handle));
        DWORD pid = 0;
        wchar_t className[128]{};
        if (!GetWindowThreadProcessId(window, &pid) || pid != expectedPid ||
            !GetClassNameW(window, className, 128) ||
            std::wcscmp(className, L"LensControlledAuthorityFixtureV1")) throw_hresult(E_ACCESSDENIED);
        // This observed class/PID check is scope filtering, NOT lifetime authority.
        stage = "mta";
        init_apartment(apartment_type::multi_threaded);
        initialized = true;
        {
            com_ptr<IUIAutomation> automation;
            stage = "uia_create";
            check_hresult(CoCreateInstance(__uuidof(CUIAutomation), nullptr, CLSCTX_INPROC_SERVER,
                __uuidof(IUIAutomation), automation.put_void()));
            com_ptr<IUIAutomationElement> root, probe;
            Barrier("before-root");
            stage = "uia_root";
            check_hresult(automation->ElementFromHandle(window, root.put()));
            stage = "uia_root_runtime_id";
            rootId = RuntimeId(root.get());
            Barrier("before-capture");
            stage = "capture_support";
            if (!GraphicsCaptureSession::IsSupported()) throw_hresult(E_NOTIMPL);
            GraphicsCaptureItem item{nullptr};
            stage = "capture_item";
            auto factory = get_activation_factory<GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
            check_hresult(factory->CreateForWindow(window, guid_of<GraphicsCaptureItem>(), put_abi(item)));
            const auto size = item.Size();
            if (size.Width < 1 || size.Height < 1 || size.Width > 4096 || size.Height > 4096) throw_hresult(E_BOUNDS);
            com_ptr<ID3D11Device> device;
            com_ptr<ID3D11DeviceContext> context;
            stage = "d3d_device";
            check_hresult(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0, D3D11_SDK_VERSION,
                device.put(), nullptr, context.put()));
            auto dxgi = device.as<IDXGIDevice>();
            com_ptr<IInspectable> inspectable;
            check_hresult(CreateDirect3D11DeviceFromDXGIDevice(dxgi.get(), inspectable.put()));
            auto projectedDevice = inspectable.as<IDirect3DDevice>();
            stage = "capture_start";
            pool = Direct3D11CaptureFramePool::CreateFreeThreaded(projectedDevice,
                DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size);
            session = pool.CreateCaptureSession(item);
            session.StartCapture();
            stage = "capture_frame";
            const ULONGLONG deadline = GetTickCount64() + 5000;
            while (!(frame = pool.TryGetNextFrame())) {
                if (GetTickCount64() >= deadline) throw_hresult(HRESULT_FROM_WIN32(WAIT_TIMEOUT));
                Sleep(10);
            }
            const auto content = frame.ContentSize();
            auto access = frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
            com_ptr<ID3D11Texture2D> texture;
            check_hresult(access->GetInterface(__uuidof(ID3D11Texture2D), texture.put_void()));
            D3D11_TEXTURE2D_DESC desc{};
            texture->GetDesc(&desc);
            if (content.Width < 1 || content.Height < 1 || static_cast<UINT>(content.Width) > desc.Width ||
                static_cast<UINT>(content.Height) > desc.Height || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM) throw_hresult(E_BOUNDS);
            // Copy one actual content-center pixel, never undefined surface padding.
            D3D11_TEXTURE2D_DESC staging{};
            staging.Width = staging.Height = staging.MipLevels = staging.ArraySize = 1;
            staging.Format = desc.Format; staging.SampleDesc.Count = 1;
            staging.Usage = D3D11_USAGE_STAGING; staging.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            com_ptr<ID3D11Texture2D> pixel;
            check_hresult(device->CreateTexture2D(&staging, nullptr, pixel.put()));
            const UINT x = static_cast<UINT>(content.Width / 2), y = static_cast<UINT>(content.Height / 2);
            D3D11_BOX box{x, y, 0, x + 1, y + 1, 1};
            context->CopySubresourceRegion(pixel.get(), 0, 0, 0, 0, texture.get(), 0, &box);
            D3D11_MAPPED_SUBRESOURCE mapped{};
            stage = "pixel_map";
            check_hresult(context->Map(pixel.get(), 0, D3D11_MAP_READ, 0, &mapped));
            const auto* bytes = static_cast<const unsigned char*>(mapped.pData);
            blue = bytes[0]; green = bytes[1]; red = bytes[2]; alpha = bytes[3];
            context->Unmap(pixel.get(), 0);
            width = content.Width; height = content.Height; sampled = true;
            Barrier("before-probe");
            stage = "uia_probe";
            check_hresult(automation->ElementFromHandle(window, probe.put()));
            stage = "uia_probe_runtime_id";
            probeId = RuntimeId(probe.get());
            BOOL equal = FALSE;
            stage = "uia_compare";
            check_hresult(automation->CompareElements(root.get(), probe.get(), &equal));
            equalJson = equal ? "true" : "false";
            Barrier("before-commit");
            stage = "observed";
        }
    } catch (const hresult_error& error) { outcome = error.code(); }
      catch (...) { outcome = E_FAIL; }
    try { if (frame) frame.Close(); if (session) session.Close(); if (pool) pool.Close(); }
    catch (...) { outcome = E_FAIL; stage = "cleanup"; }
    frame = nullptr; session = nullptr; pool = nullptr;
    if (initialized) uninit_apartment();
    const char* marker = sampled && red == 30 && green == 80 && blue == 220 ? "A"
        : sampled && red == 40 && green == 210 && blue == 70 ? "B" : "unknown";
    const int written = std::printf("{\"version\":1,\"scope\":\"isolated-acquisition-observation\",\"stage\":\"%s\","
        "\"hresult\":%ld,\"elapsed_ms\":%llu,\"root_runtime_id\":%s,\"probe_runtime_id\":%s,"
        "\"equal\":%s,\"sampled\":%s,\"width\":%d,\"height\":%d,\"bgra\":[%u,%u,%u,%u],"
        "\"marker\":\"%s\",\"product_admission_granted\":false}\n",
        stage, static_cast<long>(outcome), GetTickCount64() - started, rootId.c_str(), probeId.c_str(),
        equalJson, sampled ? "true" : "false", width, height, blue, green, red, alpha, marker);
    return FAILED(outcome) || written < 0 || std::fflush(stdout) != 0 ? 1 : 0;
}
