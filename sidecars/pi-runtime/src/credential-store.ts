import type { Credential, CredentialInfo, CredentialStore } from "@earendil-works/pi-ai";

export class HostCredentialStore implements CredentialStore {
  private credential: Credential | undefined;
  private changed = false;

  constructor(
    private readonly providerId: string,
    initial?: Credential,
  ) {
    this.credential = initial;
  }

  async read(providerId: string): Promise<Credential | undefined> {
    return providerId === this.providerId ? this.credential : undefined;
  }

  async list(): Promise<readonly CredentialInfo[]> {
    return this.credential
      ? [{ providerId: this.providerId, type: this.credential.type }]
      : [];
  }

  async modify(
    providerId: string,
    fn: (current: Credential | undefined) => Promise<Credential | undefined>,
  ): Promise<Credential | undefined> {
    if (providerId !== this.providerId) throw new Error("credential provider 不匹配");
    const next = await fn(this.credential);
    if (next !== undefined && JSON.stringify(next) !== JSON.stringify(this.credential)) {
      this.credential = next;
      this.changed = true;
    }
    return this.credential;
  }

  async delete(providerId: string): Promise<void> {
    if (providerId === this.providerId) this.credential = undefined;
  }

  takeUpdate(): Credential | undefined {
    if (!this.changed) return undefined;
    this.changed = false;
    return this.credential;
  }
}
