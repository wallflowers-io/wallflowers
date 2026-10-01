/* Types for wallflowers-client.js, the social client served beside pacific.js.
   The op names and their fields are the ICD's (wallflowers-ops.js is generated
   from it); nothing here restates them. */

export type By = 'core' | 'binding' | 'keyholder' | 'transport' | 'site';

/** Every failure: whose it was, and in whose words. `site` is only ever made by
    the site itself, before asking. */
export interface Refused { ok: false; op: string; by: By; why: string }

/** An open connection to the keyholder, e.g. from Pacific.connect({keyholder, site}). */
export interface Link { call(method: string, args?: unknown): Promise<unknown> }

/** `on`: the object kinds the door will write this op onto. */
export interface OpCapability { reachable: boolean; why: string | null; needs: string | null; on: string[] }
export interface Capabilities { writes: boolean; ops: Record<string, OpCapability> }

export interface ObjectRecovery {
  named: boolean; speakable: boolean;
  first_epoch: number | null; current_epoch: number | null;
  unreadable_epochs: number[];
}
export interface AccountRecovery {
  tail: 'intact' | 'missing' | 'forked' | 'unknown';
  holes: number[]; extra: number; found: number | null;
}

export interface FoldedObject {
  id: string; kind: string; kindInferred: boolean; name: string;
  owner: string | null; members: string[]; digest: string | null;
  messages: { author: string; text: string; ts: number; gen: number }[];
  /** The kind's view, as the core folds it. */
  view: Record<string, unknown>;
  recovery: ObjectRecovery | null;
}
export interface Model {
  me: string | null; identity: string | null; peers: unknown[];
  groups: FoldedObject[];
  problems: { what: string; why: string; id: string }[];
  version: number | undefined;
  recovery: AccountRecovery | null;
  /** True when this came from sim(): nothing in it is real. */
  fixture: boolean;
}

export interface SocialApi {
  capabilities(): Promise<Capabilities | Refused>;
  fold(group?: string): Promise<Model | Refused>;
  mint(kind: string, draft: Record<string, unknown>): Promise<{ ok: true; group: string } | Refused>;
  author(group: string, op: string, args: Record<string, unknown>): Promise<{ ok: true; deltaId: string; group: string } | Refused>;
}

export const METHOD: { capabilities: 'ops.capabilities'; fold: 'object.fold'; mint: 'object.mint'; author: 'object.author' };
export function socialApi(connect: () => Promise<Link>, options?: { normalise?: (raw: unknown) => Model }): SocialApi;
export function refusalOf(op: string, method: string, err: unknown, atConnect?: boolean): Refused;
export function isRefused(v: unknown): v is Refused;
export function siteRefusal(op: string, why: string): Refused;
/** A connection that answers object.fold from a producer fold answer and refuses every write,
    as `site`: nothing is asked of WallFlowers. */
export function sim(answer: unknown): () => Promise<Link>;
