// Lens compatibility patch for the pinned Codex ACP distribution. Only our explicit publisher
// is eligible; arbitrary tool text and links must never become HTML resources.
// Invoked by the pinned upstream call sites added during managed installation.
// eslint-disable-next-line no-unused-vars
function lensHtmlToolContent(item) {
  if (
    item.server !== "lens_output" ||
    item.tool !== "publish_html" ||
    item.status !== "completed" ||
    item.error != null ||
    item.result?.isError === true ||
    !Array.isArray(item.result?.content)
  )
    return {};
  const content = item.result.content
    .filter((block) => {
      const resource = block?.type === "resource" ? block.resource : undefined;
      return (
        resource &&
        resource.mimeType === "text/html" &&
        typeof resource.text === "string" &&
        resource.text.trim().length > 0 &&
        Buffer.byteLength(resource.text, "utf8") <= 512 * 1024 &&
        typeof resource.uri === "string" &&
        /^urn:lens:html:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(
          resource.uri,
        )
      );
    })
    .slice(0, 1)
    .map((block) => ({
      type: "content",
      content: {
        type: "resource",
        resource: {
          uri: block.resource.uri,
          mimeType: "text/html",
          text: block.resource.text,
        },
      },
    }));
  return content.length ? { content } : {};
}
