import notifications from "./acp-media-hero.json";
import type {
  DocumentBlock,
  SessionDocument,
  SessionView,
  DeferredDocumentBlock,
} from "../../src/contracts/session-document";
import type {
  LensState,
  LensOutputBlock,
  LensResponseBlockDescriptor,
} from "../../src/contracts/lens";
export const mediaCases = ["single", "multiple", "mixed", "none"] as const;
export type MediaCase = (typeof mediaCases)[number];
export const artifact =
  "<!doctype html><html><head><style>body{margin:0;background:#16324f;color:#fff;font:24px system-ui;padding:24px}strong{color:#ffdc73}</style></head><body><strong>Styled HTML</strong><p>HTML and images share the media hero.</p></body></html>";
export function mediaFixture(choice: MediaCase) {
  const images: DocumentBlock[] = notifications
    .slice(0, choice === "single" ? 1 : 3)
    .map(({ params }) => ({
      type: "image",
      mime_type: "image/png",
      data: params.update.content.data!,
    }));
  const media = choice === "none" ? [] : images;
  const content: DocumentBlock[] = [
    ...media,
    ...(choice === "mixed" ? [{ type: "html" as const, text: artifact }] : []),
    {
      type: "markdown",
      text: "## Shared answer\n\nThe same narrative follows the media.\n\n- First detail\n- Second detail",
    },
  ];
  const document: SessionDocument = {
    entries: [
      {
        id: "user",
        kind: "message",
        role: "user",
        blocks: [{ type: "markdown", text: "Show the fixture" }],
      },
      { id: "answer", kind: "message", role: "assistant", blocks: content },
    ],
  };
  const blocks: LensOutputBlock[] = content.map((block) =>
    block.type === "html"
      ? {
          type: "html",
          resource_id: "fixture-html",
          mime_type: "text/html",
          uri: "urn:lens:fixture",
          byte_length: new TextEncoder().encode(block.text).length,
        }
      : (block as LensOutputBlock),
  );
  const descriptors: LensResponseBlockDescriptor[] = blocks.map((block, block_index) => {
    switch (block.type) {
      case "markdown":
        return {
          type: "markdown",
          block_index,
          byte_length: new TextEncoder().encode(block.text).length,
        };
      case "image":
        return {
          type: "image",
          block_index,
          mime_type: block.mime_type,
          byte_length: block.data.length,
        };
      case "html":
        return { ...block, block_index };
      case "unsupported":
        return { ...block, block_index };
    }
  });
  const response = {
    prompt_execution_revision: 0,
    representation_id: `fixture-${choice}`,
    context_id: "fixture",
    context_revision: 1,
    projection: { revision: 1, digest: "fixture" },
    run_id: "fixture",
  };
  const lens: LensState = {
    operation_id: "fixture",
    stage: "completed",
    prompt_execution_revision: 0,
    output_blocks: [],
    representation: { ...response, output_blocks: [] },
    response_history: {
      responses: [
        {
          ...response,
          sequence: 1,
          blocks: descriptors,
          block_count: descriptors.length,
          retained_bytes: 0,
        },
      ],
      retained_bytes: 0,
      capacity_reached: false,
    },
  };
  const interpretation: SessionView["interpretation"] = {
    responses: [
      {
        response_id: response.representation_id,
        sequence: 1,
        blocks: descriptors.map((descriptor) => ({
          ...descriptor,
          source: {
            type: "deferred",
            entry_id: "answer",
            block_index: descriptor.block_index,
            content_type: descriptor.type,
            byte_length: "byte_length" in descriptor ? descriptor.byte_length : 0,
            revision: 1,
          },
        })),
      },
    ],
  };
  const loadSessionBlock = async (source: DeferredDocumentBlock): Promise<DocumentBlock> =>
    content[source.block_index]!;
  const port = {
    getResponseBlock: async (
      _operation: string,
      _response: string,
      index: number,
    ): Promise<LensOutputBlock> => blocks[index]!,
  };
  return { document, lens, interpretation, loadSessionBlock, port };
}
