declare module "virtual:lens-html-math-assets" {
  export const htmlMathAssets: {
    readonly resourceDigest: string;
    readonly stylesheetPath: string;
    readonly fontPaths: readonly string[];
    readonly files: readonly {
      readonly path: string;
      readonly sha256: string;
      readonly mime: "text/css" | "font/woff2";
      readonly byteLength: number;
    }[];
  };
}
