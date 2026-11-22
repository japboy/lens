#define NOMINMAX
#include <windows.h>
#include <UIAutomation.h>
#include <d3d11.h>
#include <dwmapi.h>
#include <wincodec.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <atomic>
#include <charconv>
#include <cstdio>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

using namespace winrt;
using namespace winrt::Windows::Graphics::Capture;
using namespace winrt::Windows::Graphics::DirectX;
using namespace winrt::Windows::Graphics::DirectX::Direct3D11;

namespace {
std::atomic<HWND> watched{};
std::atomic_bool destroyed{false};
void CALLBACK OnEvent(HWINEVENTHOOK, DWORD, HWND window, LONG object, LONG child, DWORD, DWORD time) {
    if (window == watched.load() && object == OBJID_WINDOW && child == CHILDID_SELF &&
        !destroyed.exchange(true)) {
        std::printf("{\"event\":\"native-destroy\",\"os_event_tick\":%lu,\"delivery_tick\":%llu}\n",
            time, GetTickCount64());
        std::fflush(stdout);
    }
}
std::jthread Watch(HWND window, DWORD pid, std::atomic<DWORD>& status) {
    watched = window;
    return std::jthread([pid, &status](std::stop_token stop) {
        MSG message{};
        PeekMessageW(&message, nullptr, 0, 0, PM_NOREMOVE);
        const auto hook = SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY, nullptr,
            OnEvent, pid, 0, WINEVENT_OUTOFCONTEXT);
        status = hook ? ERROR_SUCCESS : (GetLastError() ? GetLastError() : ERROR_GEN_FAILURE);
        if (!hook) return;
        while (!stop.stop_requested()) {
            MsgWaitForMultipleObjects(0, nullptr, FALSE, 20, QS_ALLINPUT);
            while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        const BOOL removed = UnhookWinEvent(hook);
        std::printf("{\"event\":\"hook-removed\",\"ok\":%s}\n", removed ? "true" : "false");
        std::fflush(stdout);
    });
}
uint64_t Number(std::string_view value, int radix) {
    uint64_t result = 0;
    const auto parsed = std::from_chars(value.data(), value.data() + value.size(), result, radix);
    if (parsed.ec != std::errc{} || parsed.ptr != value.data() + value.size() || !result) throw_hresult(E_INVALIDARG);
    return result;
}
std::string Json(BSTR value) {
    if (!value) return "null";
    const auto length = static_cast<int>(SysStringLen(value));
    if (length > 256) throw_hresult(E_BOUNDS);
    const int count = WideCharToMultiByte(CP_UTF8, 0, value, length, nullptr, 0, nullptr, nullptr);
    std::string bytes(static_cast<size_t>(count), '\0');
    if (count) WideCharToMultiByte(CP_UTF8, 0, value, length, bytes.data(), count, nullptr, nullptr);
    std::string result = "\"";
    for (auto c : bytes) {
        if (c == '"' || c == '\\') result += '\\';
        if (static_cast<unsigned char>(c) < 32) {
            char escape[7]{};
            std::snprintf(escape, sizeof(escape), "\\u%04x", static_cast<unsigned>(static_cast<unsigned char>(c)));
            result += escape;
        } else result += c;
    }
    return result + '"';
}
std::string Rect(RECT value) {
    return '[' + std::to_string(value.left) + ',' + std::to_string(value.top) + ',' +
        std::to_string(value.right) + ',' + std::to_string(value.bottom) + ']';
}
std::string Node(IUIAutomationElement* element, HRESULT& nameStatus) {
    BSTR name = nullptr;
    nameStatus = element->get_CurrentName(&name);
    struct Cleanup { BSTR value; ~Cleanup() { SysFreeString(value); } } cleanup{name};
    if (FAILED(nameStatus)) return "null";
    CONTROLTYPEID role = 0;
    RECT bounds{};
    const auto roleStatus = element->get_CurrentControlType(&role);
    const auto boundsStatus = element->get_CurrentBoundingRectangle(&bounds);
    return "{\"name\":" + Json(name) + ",\"role\":" + std::to_string(role) +
        ",\"role_hr\":" + std::to_string(roleStatus) + ",\"bounds_hr\":" + std::to_string(boundsStatus) +
        ",\"bounds\":" + Rect(bounds) + '}';
}
std::string Tree(IUIAutomation* automation, IUIAutomationElement* root, HRESULT& status) {
    const auto rootJson = Node(root, status);
    if (FAILED(status)) return "null";
    com_ptr<IUIAutomationTreeWalker> walker;
    check_hresult(automation->get_RawViewWalker(walker.put()));
    com_ptr<IUIAutomationElement> child;
    check_hresult(walker->GetFirstChildElement(root, child.put()));
    std::string result = "{\"root\":" + rootJson + ",\"children\":[";
    unsigned index = 0;
    for (; child && index < 8; ++index) {
        HRESULT childStatus = S_OK;
        if (index) result += ',';
        result += Node(child.get(), childStatus);
        com_ptr<IUIAutomationElement> next;
        check_hresult(walker->GetNextSiblingElement(child.get(), next.put()));
        child = std::move(next);
    }
    return result + "],\"truncated\":" + (child ? "true" : "false") + '}';
}
struct Image {
    UINT width, height;
    std::vector<BYTE> bytes;
};
Image CopyFrame(Direct3D11CaptureFrame const& frame, ID3D11Device* device, ID3D11DeviceContext* context) {
    const auto size = frame.ContentSize();
    auto access = frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
    com_ptr<ID3D11Texture2D> texture;
    check_hresult(access->GetInterface(__uuidof(ID3D11Texture2D), texture.put_void()));
    D3D11_TEXTURE2D_DESC desc{};
    texture->GetDesc(&desc);
    if (size.Width < 1 || size.Height < 1 || size.Width > 4096 || size.Height > 4096 ||
        static_cast<UINT>(size.Width) > desc.Width || static_cast<UINT>(size.Height) > desc.Height ||
        desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM) throw_hresult(E_BOUNDS);
    Image image{static_cast<UINT>(size.Width), static_cast<UINT>(size.Height), {}};
    const UINT stride = image.width * 4;
    image.bytes.resize(stride * image.height);
    D3D11_TEXTURE2D_DESC staging{};
    staging.Width = image.width; staging.Height = image.height;
    staging.MipLevels = staging.ArraySize = staging.SampleDesc.Count = 1;
    staging.Format = desc.Format;
    staging.Usage = D3D11_USAGE_STAGING;
    staging.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    com_ptr<ID3D11Texture2D> copy;
    check_hresult(device->CreateTexture2D(&staging, nullptr, copy.put()));
    const D3D11_BOX area{0, 0, 0, image.width, image.height, 1};
    context->CopySubresourceRegion(copy.get(), 0, 0, 0, 0, texture.get(), 0, &area);
    D3D11_MAPPED_SUBRESOURCE mapped{};
    check_hresult(context->Map(copy.get(), 0, D3D11_MAP_READ, 0, &mapped));
    for (UINT y = 0; y < image.height; ++y) {
        std::memcpy(image.bytes.data() + y * stride,
            static_cast<const BYTE*>(mapped.pData) + y * mapped.RowPitch, stride);
    }
    context->Unmap(copy.get(), 0);
    return image;
}
void Png(Image& image, const std::wstring& filename) {
    com_ptr<IWICImagingFactory> factory;
    check_hresult(CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(factory.put())));
    com_ptr<IWICStream> stream;
    check_hresult(factory->CreateStream(stream.put()));
    check_hresult(stream->InitializeFromFilename(filename.c_str(), GENERIC_WRITE));
    com_ptr<IWICBitmapEncoder> encoder;
    check_hresult(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, encoder.put()));
    check_hresult(encoder->Initialize(stream.get(), WICBitmapEncoderNoCache));
    com_ptr<IWICBitmapFrameEncode> frame;
    check_hresult(encoder->CreateNewFrame(frame.put(), nullptr));
    check_hresult(frame->Initialize(nullptr));
    check_hresult(frame->SetSize(image.width, image.height));
    auto pixel = GUID_WICPixelFormat32bppBGRA;
    check_hresult(frame->SetPixelFormat(&pixel));
    if (pixel != GUID_WICPixelFormat32bppBGRA) throw_hresult(E_UNEXPECTED);
    check_hresult(frame->WritePixels(image.height, image.width * 4,
        static_cast<UINT>(image.bytes.size()), image.bytes.data()));
    check_hresult(frame->Commit());
    check_hresult(encoder->Commit());
}
}

int main(int argc, char** argv) {
    const char* stage = "input";
    try {
        if (argc != 4 || std::strncmp(argv[1], "0x", 2)) throw_hresult(E_INVALIDARG);
        const uint64_t handle = Number(argv[1] + 2, 16), expectedPid = Number(argv[2], 10);
        if (handle > UINTPTR_MAX || expectedPid > MAXDWORD) throw_hresult(E_INVALIDARG);
        const std::string channel = argv[3];
        if (channel.length() != 32 || channel.find_first_not_of("0123456789abcdef") != std::string::npos)
            throw_hresult(E_INVALIDARG);
        const auto window = reinterpret_cast<HWND>(static_cast<uintptr_t>(handle));
        if (!SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)) throw_last_error();
        DWORD pid = 0;
        wchar_t name[128]{};
        if (!GetWindowThreadProcessId(window, &pid) || pid != expectedPid ||
            !GetClassNameW(window, name, 128) || std::wcscmp(name, L"LensControlledAuthorityFixtureV1") ||
            GetAncestor(window, GA_ROOT) != window || GetWindow(window, GW_OWNER))
            throw_hresult(E_ACCESSDENIED);
        init_apartment(apartment_type::multi_threaded);
        struct Apartment { ~Apartment() { uninit_apartment(); } } apartment;
        stage = "watch";
        std::atomic<DWORD> status{MAXDWORD};
        auto monitor = Watch(window, pid, status);
        const auto watchDeadline = GetTickCount64() + 1000;
        while (status == MAXDWORD && GetTickCount64() < watchDeadline) Sleep(5);
        if (status != ERROR_SUCCESS) throw_hresult(HRESULT_FROM_WIN32(status == MAXDWORD ? WAIT_TIMEOUT : status.load()));
        std::printf("{\"event\":\"watch-ready\",\"channel\":\"%s\"}\n", channel.c_str());
        std::fflush(stdout);
        stage = "bind-command";
        char line[64]{};
        if (!std::fgets(line, sizeof(line), stdin) || std::strcmp(line, "bind\n")) throw_hresult(E_ABORT);
        if (destroyed) throw_hresult(HRESULT_FROM_WIN32(ERROR_INVALID_WINDOW_HANDLE));
        stage = "uia-bind";
        com_ptr<IUIAutomation> automation;
        check_hresult(CoCreateInstance(__uuidof(CUIAutomation), nullptr, CLSCTX_INPROC_SERVER,
            IID_PPV_ARGS(automation.put())));
        com_ptr<IUIAutomationElement> root;
        check_hresult(automation->ElementFromHandle(window, root.put()));
        HRESULT rootStatus = S_OK;
        const auto boundRoot = Node(root.get(), rootStatus);
        check_hresult(rootStatus);
        UIA_HWND rootWindow = nullptr;
        check_hresult(root->get_CurrentNativeWindowHandle(&rootWindow));
        if (rootWindow != window) throw_hresult(E_UNEXPECTED);
        std::printf("{\"event\":\"uia-bound\",\"channel\":\"%s\",\"root_hwnd\":\"0x%llx\",\"root\":%s}\n",
            channel.c_str(), static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(rootWindow)), boundRoot.c_str());
        std::fflush(stdout);
        stage = "wgc-command";
        if (!std::fgets(line, sizeof(line), stdin) || std::strcmp(line, "continue-wgc\n")) throw_hresult(E_ABORT);
        if (destroyed) throw_hresult(HRESULT_FROM_WIN32(ERROR_INVALID_WINDOW_HANDLE));
        stage = "wgc-bind";
        GraphicsCaptureItem item{nullptr};
        auto factory = get_activation_factory<GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
        check_hresult(factory->CreateForWindow(window, guid_of<GraphicsCaptureItem>(), put_abi(item)));
        auto closed = std::make_shared<std::atomic_bool>(false);
        auto event = item.Closed(auto_revoke, [closed](auto const&, auto const&) noexcept {
            closed->store(true);
            std::printf("{\"event\":\"wgc-closed\",\"delivery_tick\":%llu}\n", GetTickCount64());
            std::fflush(stdout);
        });
        const auto size = item.Size();
        if (size.Width < 1 || size.Height < 1 || size.Width > 4096 || size.Height > 4096) throw_hresult(E_BOUNDS);
        const UINT dpi = GetDpiForWindow(window);
        stage = "capture-setup";
        com_ptr<ID3D11Device> device;
        com_ptr<ID3D11DeviceContext> context;
        check_hresult(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0, D3D11_SDK_VERSION,
            device.put(), nullptr, context.put()));
        com_ptr<IInspectable> inspectable;
        check_hresult(CreateDirect3D11DeviceFromDXGIDevice(device.as<IDXGIDevice>().get(), inspectable.put()));
        auto pool = Direct3D11CaptureFramePool::CreateFreeThreaded(inspectable.as<IDirect3DDevice>(),
            DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size);
        auto session = pool.CreateCaptureSession(item);
        session.IsCursorCaptureEnabled(false);
        bool captureStarted = false;
        std::printf("{\"event\":\"bound\",\"channel\":\"%s\",\"dpi\":%u}\n", channel.c_str(), dpi);
        std::fflush(stdout);
        int64_t lastTime = -1;
        uint64_t lastRead = 0;
        stage = "read-command";
        while (std::fgets(line, sizeof(line), stdin) && std::strcmp(line, "stop\n")) {
            const std::string_view request(line);
            if (request == "inspect\n") {
                BSTR retainedName = nullptr;
                const HRESULT hr = root->get_CurrentName(&retainedName);
                SysFreeString(retainedName);
                std::printf("{\"event\":\"retained-check\",\"uia_hr\":%ld,\"wgc_closed\":%s,\"native_destroy\":%s}\n",
                    hr, closed->load() ? "true" : "false", destroyed ? "true" : "false");
                std::fflush(stdout);
                continue;
            }
            if (!request.starts_with("read ") || !request.ends_with('\n') || lastRead >= 8) throw_hresult(E_INVALIDARG);
            auto readCommand = request.substr(5, request.size() - 6);
            constexpr std::string_view stallSuffix = " stall";
            constexpr std::string_view lateSuffix = " late";
            const bool stall = readCommand.ends_with(stallSuffix);
            const bool late = readCommand.ends_with(lateSuffix);
            if (stall) readCommand.remove_suffix(stallSuffix.size());
            if (late) readCommand.remove_suffix(lateSuffix.size());
            const auto read = Number(readCommand, 10);
            if (read != lastRead + 1) throw_hresult(E_INVALIDARG);
            lastRead = read;
            if (destroyed || closed->load()) {
                std::printf("{\"event\":\"read-blocked\",\"reason\":\"native-invalidation\",\"read\":%llu}\n", read);
                std::fflush(stdout);
                continue;
            }
            stage = "uia-tree";
            RECT before{}, after{};
            const auto rectHr = DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &before, sizeof(before));
            HRESULT uiaHr = S_OK;
            const auto tree = Tree(automation.get(), root.get(), uiaHr);
            if (FAILED(uiaHr)) {
                std::printf("{\"event\":\"uia-failure\",\"uia_hr\":%ld,\"read\":%llu}\n", uiaHr, read);
                std::fflush(stdout);
                continue;
            }
            if (!captureStarted) {
                session.StartCapture();
                captureStarted = true;
                std::printf("{\"event\":\"capture-started\",\"channel\":\"%s\"}\n", channel.c_str());
                std::fflush(stdout);
            }
            if (stall || late) {
                stage = "validation-wait";
                std::printf("{\"event\":\"validation-wait\",\"channel\":\"%s\",\"read\":\"%llu\",\"mode\":\"%s\",\"tick\":%llu}\n",
                    channel.c_str(), read, stall ? "stall" : "late", GetTickCount64());
                std::fflush(stdout);
                if (stall) {
                    // No native call is made unbounded. This controlled delay outlasts the host budget.
                    Sleep(30000);
                    throw_hresult(HRESULT_FROM_WIN32(WAIT_TIMEOUT));
                }
                char resume[32]{};
                if (!std::fgets(resume, sizeof(resume), stdin) || std::strcmp(resume, "resume\n"))
                    throw_hresult(E_ABORT);
            }
            stage = "capture-frame";
            Image image{};
            int64_t currentTime = -1;
            const auto deadline = GetTickCount64() + 3500;
            ULONGLONG settle = 0;
            while (GetTickCount64() < deadline && (!settle || GetTickCount64() < settle) &&
                !destroyed && !closed->load()) {
                auto frame = pool.TryGetNextFrame();
                if (frame) {
                    const auto time = frame.SystemRelativeTime().count();
                    if (time > lastTime && time > currentTime) {
                        image = CopyFrame(frame, device.get(), context.get());
                        currentTime = time;
                        if (!settle) settle = GetTickCount64() + 180;
                    }
                    frame.Close();
                } else Sleep(10);
            }
            if (destroyed || closed->load()) {
                std::printf("{\"event\":\"read-blocked\",\"reason\":\"native-invalidation\",\"read\":%llu}\n", read);
                std::fflush(stdout);
                continue;
            }
            if (currentTime < 0) throw_hresult(HRESULT_FROM_WIN32(WAIT_TIMEOUT));
            lastTime = currentTime;
            const auto afterHr = DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &after, sizeof(after));
            stage = "png";
            Png(image, L"read-" + std::to_wstring(read) + L".png");
            const auto* center = image.bytes.data() + (image.width * (image.height / 2) + image.width / 2) * 4;
            std::printf("{\"event\":\"result\",\"channel\":\"%s\",\"read\":\"%llu\",\"frame_time_100ns\":\"%lld\","
                "\"width\":%u,\"height\":%u,\"center_bgra\":[%u,%u,%u,%u],\"dwm_before_hr\":%ld,\"dwm_after_hr\":%ld,"
                "\"dwm_before\":%s,\"dwm_after\":%s,\"uia\":%s}\n",
                channel.c_str(), read, currentTime, image.width, image.height,
                center[0], center[1], center[2], center[3], rectHr, afterHr,
                Rect(before).c_str(), Rect(after).c_str(), tree.c_str());
            std::fflush(stdout);
            stage = "read-command";
        }
        session.Close();
        pool.Close();
        std::printf("{\"event\":\"worker-cleanup\"}\n");
        return 0;
    } catch (const hresult_error& error) {
        std::printf("{\"event\":\"worker-error\",\"stage\":\"%s\",\"hresult\":%ld}\n",
            stage, static_cast<long>(error.code()));
        return 1;
    }
}
