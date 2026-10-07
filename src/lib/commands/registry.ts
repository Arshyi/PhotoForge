/**
 * The command registry: everything the command palette can run.
 *
 * Built-in commands and plugin commands are entries in one list, with one shape, so
 * the palette, the shortcut hints and the automation editor all draw from the same
 * place. A command is a *name for something the interface already does* — it carries
 * no editing logic of its own. Anything that changes the document does it by asking
 * the backend to run registered operations, exactly as the Layers panel does.
 *
 * Plugins can add commands and can never take a built-in's name: a plugin command
 * is always `plugin:<plugin id>:<command id>`, built from the ids the backend
 * validated, and the registry refuses an entry that claims a namespace it does not
 * own.
 */
export type CommandSource = 'core' | 'plugin' | 'macro';

export interface PaletteCommand {
  /** `core.<name>`, `plugin:<plugin id>:<command id>` or `macro:<macro id>`. */
  id: string;
  title: string;
  description?: string;
  /** The heading the palette groups it under. */
  group: string;
  /** The key binding that does the same, to show beside it. */
  shortcut?: string;
  /** Extra words that should find it. */
  keywords?: string[];
  source: CommandSource;
  /** A sentence saying why it cannot run now, or `null` when it can. */
  unavailable: () => string | null;
  run: () => void | Promise<void>;
}

export class CommandRegistry {
  private readonly entries = new Map<string, PaletteCommand>();

  /** Adds a command. Refuses a repeated id and a namespace the source does not own. */
  register(command: PaletteCommand): void {
    if (this.entries.has(command.id)) {
      throw new Error(`There is already a command with the id ${command.id}.`);
    }
    if (command.source === 'plugin' && !command.id.startsWith('plugin:')) {
      throw new Error(`A plugin command must be called plugin:<plugin>:<command>, not ${command.id}.`);
    }
    if (command.source === 'macro' && !command.id.startsWith('macro:')) {
      throw new Error(`A macro command must be called macro:<macro>, not ${command.id}.`);
    }
    if (command.source === 'core' && !command.id.startsWith('core.')) {
      throw new Error(`A built-in command must be called core.<name>, not ${command.id}.`);
    }
    if (!command.title.trim() || command.title.length > 120) {
      throw new Error(`Command ${command.id} needs a title of 1 to 120 characters.`);
    }
    this.entries.set(command.id, command);
  }

  /** Removes every command from a source, for rebuilding when plugins change. */
  clear(source?: CommandSource): void {
    for (const [id, command] of this.entries) {
      if (!source || command.source === source) this.entries.delete(id);
    }
  }

  get(id: string): PaletteCommand | undefined {
    return this.entries.get(id);
  }

  list(): PaletteCommand[] {
    return [...this.entries.values()];
  }
}

export interface ScoredCommand {
  command: PaletteCommand;
  score: number;
}

function words(text: string): string[] {
  return text.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
}

/**
 * Scores one command against a query: every word of the query must match somewhere,
 * and a match at the start of the title beats one at the start of a word, which beats
 * one inside a word, which beats a match only in the group or keywords. Zero means no match.
 */
export function scoreCommand(command: PaletteCommand, query: string): number {
  const needles = words(query);
  if (!needles.length) return 1;
  const title = command.title.toLowerCase();
  const titleWords = words(command.title);
  const other = words([command.group, ...(command.keywords ?? []), command.description ?? ''].join(' '));
  let total = 0;
  for (const needle of needles) {
    let best = 0;
    if (title.startsWith(needle)) best = 100;
    else if (titleWords.some((word) => word.startsWith(needle))) best = 70;
    else if (title.includes(needle)) best = 40;
    else if (other.some((word) => word.startsWith(needle))) best = 20;
    else if (other.some((word) => word.includes(needle))) best = 8;
    if (best === 0) return 0;
    total += best;
  }
  // Shorter titles read as closer matches, and ties keep their registration order.
  return total - Math.min(title.length, 60) / 100;
}

/**
 * The commands a query finds, best first. With no query, everything, in the order
 * registered. Commands that cannot run now are still listed — with the reason — so
 * a person learns why instead of wondering where a command went.
 */
export function searchCommands(commands: PaletteCommand[], query: string, limit = 50): ScoredCommand[] {
  return commands
    .map((command, index) => ({ command, score: scoreCommand(command, query), index }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .slice(0, limit)
    .map(({ command, score }) => ({ command, score }));
}

/** The id a plugin's command has in the registry. */
export function macroCommandId(macro: string): string {
  return `macro:${macro}`;
}

export function pluginCommandId(plugin: string, command: string): string {
  return `plugin:${plugin}:${command}`;
}
