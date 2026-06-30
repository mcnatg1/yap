import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const source = path.resolve(
  "C:/Users/mcnatg1/.cursor/plugins/cache/cursor-public/shadcn/10f1717a3e2a3c16cfbd43877c1e44063d9d749a/apps/v4/registry/new-york-v4/ui/sidebar.tsx",
);
const target = fileURLToPath(new URL("../src/components/ui/sidebar.tsx", import.meta.url));

let content = fs.readFileSync(source, "utf8");
content = content
  .replaceAll("@/registry/new-york-v4/hooks/use-mobile", "@/hooks/use-mobile")
  .replaceAll("@/registry/new-york-v4/lib/utils", "@/lib/utils")
  .replaceAll("@/registry/new-york-v4/ui/", "@/components/ui/")
  .replace("PanelLeftIcon", "PanelLeft");

fs.writeFileSync(target, content);
console.log(`Wrote ${target} (${content.split("\n").length} lines)`);
