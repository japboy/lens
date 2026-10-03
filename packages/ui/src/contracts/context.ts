export const DESKTOP_PLATFORMS = ["macos", "windows", "linux"] as const;
export type DesktopPlatform = (typeof DESKTOP_PLATFORMS)[number];
