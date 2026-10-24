import { execSync } from "child_process";

export default function globalSetup() {
  execSync("npx tsx e2e/seed-db.ts", { stdio: "inherit" });
}
