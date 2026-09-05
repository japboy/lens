#!/usr/bin/env node
//MISE description = "Check the application release version authority and all explicit mirrors"
//MISE dir = "{{config_root}}"

import { fileURLToPath } from "node:url";
import { readVersion } from "../../scripts/release/version.ts";

process.stdout.write(
  `${JSON.stringify(readVersion(fileURLToPath(new URL("../../", import.meta.url))))}\n`,
);
