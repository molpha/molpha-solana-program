import { Command } from "commander";
import { registerUpdateConfigCommand } from "./update-config";

export function registerAdminCommands(program: Command): void {
  const admin = program.command("admin").description("Admin operations");

  registerUpdateConfigCommand(admin);
}
