export type Request = <T>(path: string, method?: string, body?: unknown) => Promise<T>;
export type GitHub = {
  request: Request;
  upload: (url: string, name: string, bytes: Buffer) => Promise<void>;
  download: (path: string) => Promise<Buffer>;
};

export class ApiError extends Error {
  status: number;
  constructor(status: number, method: string, path: string) {
    super(`GitHub ${method} ${path.split("?")[0]} returned ${status}`);
    this.status = status;
  }
}

export function github(token: string, repository: string): GitHub {
  if (!token || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repository))
    throw new Error("Explicit repository and token required");
  const base = `https://api.github.com/repos/${repository}`;
  const headers = { Authorization: `Bearer ${token}`, "X-GitHub-Api-Version": "2022-11-28" };
  const request: Request = async <T>(path: string, method = "GET", body?: unknown): Promise<T> => {
    if (!path.startsWith("/") || path.startsWith("//"))
      throw new Error("Repository-relative API path required");
    const response = await fetch(base + path, {
      method,
      headers: {
        ...headers,
        Accept: "application/vnd.github+json",
        "Content-Type": "application/json",
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) throw new ApiError(response.status, method, path);
    return (response.status === 204 ? undefined : await response.json()) as T;
  };
  return {
    request,
    upload: async (url, name, bytes) => {
      const target = new URL(url.replace(/\{.*$/u, ""));
      if (
        target.origin !== "https://uploads.github.com" ||
        !target.pathname.startsWith(`/repos/${repository}/releases/`)
      )
        throw new Error("Untrusted release upload URL");
      target.searchParams.set("name", name);
      const response = await fetch(target, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/octet-stream" },
        body: new Uint8Array(bytes),
        signal: AbortSignal.timeout(60_000),
      });
      if (!response.ok) throw new ApiError(response.status, "POST", target.pathname);
    },
    download: async (path) => {
      if (!/^\/releases\/assets\/\d+$/u.test(path)) throw new Error("Invalid asset endpoint");
      const response = await fetch(base + path, {
        headers: { ...headers, Accept: "application/octet-stream" },
        signal: AbortSignal.timeout(60_000),
      });
      if (!response.ok) throw new ApiError(response.status, "GET", path);
      return Buffer.from(await response.arrayBuffer());
    },
  };
}

export async function optional<T>(read: () => Promise<T>): Promise<T | undefined> {
  try {
    return await read();
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return undefined;
    throw error;
  }
}

export async function pages<T>(request: Request, path: string): Promise<T[]> {
  const result: T[] = [];
  for (let page = 1; ; page++) {
    const entries = await request<T[]>(
      `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
    );
    if (!Array.isArray(entries)) throw new Error("Expected paginated API list");
    result.push(...entries);
    if (entries.length < 100) return result;
  }
}
