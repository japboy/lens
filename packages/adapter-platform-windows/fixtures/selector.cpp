#include <windows.h>
#include <cstdio>
#include <cstring>
int main(int argc, char** argv) {
    if (argc != 2 || (std::strcmp(argv[1], "A") != 0 && std::strcmp(argv[1], "B") != 0)) return 2;
    const bool a = argv[1][0] == 'A';
    const auto result = MessageBoxW(nullptr,
        a ? L"Confirm Lens fixture A for the first structured and image read?"
          : L"Confirm Lens fixture B for a new structured and image read?",
        a ? L"Lens #207 - Confirm leased fixture A" : L"Lens #207 - Confirm leased fixture B",
        MB_OKCANCEL | MB_ICONINFORMATION | MB_DEFBUTTON2 | MB_SETFOREGROUND);
    if (result != IDOK && result != IDCANCEL) {
        const DWORD error = result == 0 ? GetLastError() : ERROR_SUCCESS;
        std::printf("{\"event\":\"selector-error\",\"result\":%d,\"win32_error\":%lu}\n", result, error);
        return 2;
    }
    std::printf("{\"event\":\"selection-response\",\"approved\":%s}\n", result == IDOK ? "true" : "false");
    return result == IDOK ? 0 : 1;
}
