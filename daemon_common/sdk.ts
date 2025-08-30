import * as ed from '@noble/ed25519';
import { Client as UndiciClient } from 'undici';
import * as crypto from 'node:crypto';
import { Readable } from 'node:stream';
import * as forge from 'node-forge';

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]?: JsonValue } | null;
export type PatchEntry = { "patchtype": "normal", patch: (
      { op: 'add', path: string, value: any }
      | { op: 'remove', path: string }
      | { op: 'replace', path: string, value: any }
      | { op: 'move', from: string, path: string }
      | { op: 'copy', from: string, path: string }
      | { op: 'test', path: string, value: any }
    ) [], } | { "patchtype": "merge", patch: JsonValue, };
export type PatchSet = { condition: (
      { op: 'add', path: string, value: any }
      | { op: 'remove', path: string }
      | { op: 'replace', path: string, value: any }
      | { op: 'move', from: string, path: string }
      | { op: 'copy', from: string, path: string }
      | { op: 'test', path: string, value: any }
    ) [], patch: PatchEntry, };
export type ServerFileWrite = { "filetype": "write", data: Array<number>, } | { "filetype": "java_properties", content: { [key in string]?: string }, } | { "filetype": "json", set: Array<PatchSet>, } | { "filetype": "yaml", set: Array<PatchSet>, };
export type ServerBuildImage = { dockerfile: string, default_tag: string | null, tag: string | null, args: { [key in string]?: string }, parents: Array<ServerBuildImage>, base: number | null, };
export type ServerBuildPayload = { image: ServerBuildImage, backup_exclude: Array<string>, files_write: { [key in string]?: ServerFileWrite }, cap_add: Array<string>, cap_drop: Array<string>, environment: { [key in string]?: string }, uid: number | null, gid: number | null, stop_command: string | null, stop_timeout: number, };
export type ServerResourcePayload = { cpu: number | null, ram: number | null, swap: number | null, disk: bigint | null, };
export type ServerPortEntryPayload = { internal_port: number, external_port: number, external_ip: string, protocol: string, };
export type NetworkAttachment = { network: number, ipv4: number, ipv6: bigint, };
export type ServerNetworkPayload = { internet_access: boolean, networks: Array<NetworkAttachment>, };
export type CreateServerError = { "Communication": any } | "auth/invalid" | "server/image_different" | "server/image_missing_tag" | "storage/location_not_found" | "sql/failure";
export type UpdateServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "server/image_different" | "server/image_missing_tag" | "sql/failure";
export type DeleteServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "server/mutex_locked" | "docker/failure" | "sql/failure";
export type PublicServerState = { label: string, healthcheck: string | null, start: boolean, stop: boolean, kill: boolean, time: bigint | null, };
export type ServerStatusError = { "Communication": any } | "auth/invalid" | "server/not_found" | "server/mutex_locked" | "docker/failure" | "sql/failure";
export type StartServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
export type StopServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
export type KillServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
export type RestartServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
export type ExecServerError = { "Communication": any } | "auth/invalid" | "server/not_found" | "server/mutex_locked" | "sql/failure";
export type WipeServerError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "sql/failure";
export type BackupResponseEntry = { time: bigint, storage: string, name: string | null, size: bigint | null, };
export type ServerListBackupsError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "sql/failure";
export type ServerCreateBackupOrderError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup_storage/no_suitable_found" | "sql/failure";
export type ServerCreateBackupError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup_storage/incompatible" | "backup_storage/location_not_found" | "sql/failure";
export type ServerDownloadBackupError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup_storage/location_not_found" | "sql/failure";
export type ServerDownloadBackupUserError = { "Communication": any } | "auth/signature_invalid" | "auth/signature_expired" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup_storage/location_not_found" | "sql/failure";
export type ServerDownloadRealtimeError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "compression/unsupported" | "sql/failure";
export type ServerDownloadRealtimeUserError = { "Communication": any } | "auth/signature_invalid" | "auth/signature_expired" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "compression/unsupported" | "sql/failure";
export type ServerSetBackupNameError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup/not_found" | "backup_storage/location_not_found" | "sql/failure";
export type ServerDeleteBackupError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "backup/not_found" | "backup_storage/location_not_found" | "sql/failure";
export type ServerRestoreBackupError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "backup/not_found" | "backup_storage/location_not_found" | "sql/failure";
export type FileEntry = { name: string, uid: number, gid: number, mode: number, size: bigint, relative_path: string, path: string | null, archive: boolean, };
export type ServerListFilesError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "file/not_directory" | "sql/failure";
export type ServerStatFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "sql/failure";
export type ServerRenameFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "file/already_exists" | "sql/failure";
export type ServerCopyFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "sql/failure";
export type ServerDownloadFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "file/limit_reached" | "sql/failure";
export type ServerDownloadFileUserError = { "Communication": any } | "auth/signature_invalid" | "auth/signature_expired" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "file/limit_reached" | "sql/failure";
export type ServerUploadFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "sql/failure";
export type ServerUploadFileUserError = { "Communication": any } | "auth/signature_invalid" | "auth/signature_expired" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "sql/failure";
export type ServerWriteFileByUrlError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "sql/failure" | "http_request/failed" | "http_request/not_success" | "http_request/read_failed";
export type ServerWriteFileByTemplateError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "storage/patch_failed" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "sql/failure";
export type ServerDeleteFileError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "file/not_found" | "file/sandbox_violated" | "file/not_directory" | "file/is_directory" | "sql/failure";
export type ServerCreateDirectoryError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/already_exists" | "sql/failure";
export type ServerSymlinkError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/already_exists" | "sql/failure";
export type ServerUnpackArchiveError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "compression/unsupported" | "sql/failure";
export type ServerCreateArchiveError = { "Communication": any } | "auth/invalid" | "storage/not_found" | "storage/mutex_locked" | "storage/failure" | "storage/quota_reached" | "file/not_found" | "file/sandbox_violated" | "file/is_directory" | "compression/unsupported" | "sql/failure";
export type CreateNetworkError = { "Communication": any } | "auth/invalid" | "network/prefix_collision" | "network/prefix_invalid" | "docker/failure" | "sql/failure";
export type DeleteNetworkError = { "Communication": any } | "auth/invalid" | "network/not_found" | "docker/failure" | "sql/failure";
export type ResourceDisk = { available: bigint, total: bigint, mountpoint: string, readonly: boolean, filesystem: string, };
export type ResourceResponse = { mem_total: bigint, mem_used: bigint, mem_allocated: bigint, swap_total: bigint, swap_used: bigint, swap_allocated: bigint, cpu_used: number, cpu_count: number, disks: Array<ResourceDisk>, };
export type ResourceInfoError = { "Communication": any } | "auth/invalid" | "sql/failure" | "ResourceError";
export type CreateDatabaseError = { "Communication": any } | "auth/invalid" | "database/invalid_identifier" | "sql/failure";
export type DatabaseUserPermissions = number;
export type DatabaseUserPayload = { password_hash: string, permissions: number, };
export type UpdateDatabaseError = { "Communication": any } | "auth/invalid" | "database/not_found" | "database/invalid_identifier" | "database/invalid_hash" | "database/permissionless_entry" | "sql/failure";
export type DeleteDatabaseError = { "Communication": any } | "auth/invalid" | "database/not_found" | "sql/failure";
export type ScheduleRunPolicy = "always" | "running" | "stopped";
export type ScheduleStep = { "type": "start" } | { "type": "stop" } | { "type": "restart" } | { "type": "kill" } | { "type": "sleep", time: bigint, } | { "type": "terminal", command: string, } | { "type": "container", command: Array<string>, };
export type SchedulePayload = { policy: ScheduleRunPolicy, schedule: string, timezone: string, steps: Array<ScheduleStep>, };
export type CreateScheduleError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
export type UpdateScheduleError = { "Communication": any } | "auth/invalid" | "server/not_found" | "schedule/not_found" | "sql/failure";
export type DeleteScheduleError = { "Communication": any } | "auth/invalid" | "server/not_found" | "sql/failure";
namespace DatabaseUserPermissions {
  const SELECT: number = 1;
  const INSERT: number = 2;
  const UPDATE: number = 4;
  const DELETE: number = 8;
  const CREATE: number = 16;
  const DROP: number = 32;
  const INDEX: number = 64;
}
namespace FileMode {
  const FIFO: number = 4096;
  const CHR: number = 8192;
  const DIR: number = 16384;
  const REG: number = 32768;
}

export class UserSignature {
    readonly signature: Uint8Array;

    constructor(signature: Uint8Array) {
        this.signature = signature;
    }

    static new(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null) {
        return new UserSignature(ed.sign(UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), privateKey));
    }

    static async newAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): Promise<UserSignature> {
        return new UserSignature(await ed.signAsync(UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), privateKey));
    }

    verify(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): boolean {
        return ed.verify(this.signature, UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), publicKey);
    }

    async verifyAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): Promise<boolean> {
        return await ed.verifyAsync(this.signature, UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), publicKey);
    }

    private static content(timestamp: bigint, node_identifier: string, server_id: number, name: string, extra: string | null): Buffer {
        return Buffer.from(`${name}-${node_identifier}-${server_id}-${timestamp}${extra === null ? "" : extra}`, 'utf8');
    }

    toString(): string {
        return Buffer.from(this.signature).toString('base64');
    }


  static newServerBackupDownload(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, backupStorage: string, backupId: bigint): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "backup", `-${backupStorage}-${backupId}`);
  }

  static async newServerBackupDownloadAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, backupStorage: string, backupId: bigint): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "backup", `-${backupStorage}-${backupId}`);
  }

  verifyServerBackupDownload(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, backupStorage: string, backupId: bigint): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "backup", `-${backupStorage}-${backupId}`);
  }

  async verifyServerBackupDownloadAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, backupStorage: string, backupId: bigint): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "backup", `-${backupStorage}-${backupId}`);
  }

  static newServerRealtimeDownload(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, compression: string): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "backup_rt", `-${compression}`);
  }

  static async newServerRealtimeDownloadAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, compression: string): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "backup_rt", `-${compression}`);
  }

  verifyServerRealtimeDownload(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, compression: string): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "backup_rt", `-${compression}`);
  }

  async verifyServerRealtimeDownloadAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, compression: string): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "backup_rt", `-${compression}`);
  }

  static newTerminal(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, write: boolean): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "terminal", `-${write}`);
  }

  static async newTerminalAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, write: boolean): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "terminal", `-${write}`);
  }

  verifyTerminal(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, write: boolean): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "terminal", `-${write}`);
  }

  async verifyTerminalAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, write: boolean): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "terminal", `-${write}`);
  }

  static newStats(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "stats", null);
  }

  static async newStatsAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "stats", null);
  }

  verifyStats(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "stats", null);
  }

  async verifyStatsAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "stats", null);
  }

  static newExec(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, command: Array<string>): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "exec", `-${command.join(";")}`);
  }

  static async newExecAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, command: Array<string>): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "exec", `-${command.join(";")}`);
  }

  verifyExec(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, command: Array<string>): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "exec", `-${command.join(";")}`);
  }

  async verifyExecAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, command: Array<string>): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "exec", `-${command.join(";")}`);
  }

  static newServerFileDownload(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "server_file_download", `-${path}`);
  }

  static async newServerFileDownloadAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "server_file_download", `-${path}`);
  }

  verifyServerFileDownload(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "server_file_download", `-${path}`);
  }

  async verifyServerFileDownloadAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "server_file_download", `-${path}`);
  }

  static newServerFileUpload(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): UserSignature {
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, "server_file_upload", `-${path}`);
  }

  static async newServerFileUploadAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): Promise<UserSignature> {
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, "server_file_upload", `-${path}`);
  }

  verifyServerFileUpload(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): boolean {
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, "server_file_upload", `-${path}`);
  }

  async verifyServerFileUploadAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, path: string): Promise<boolean> {
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, "server_file_upload", `-${path}`);
  }


}

type DaemonSDKRequest = { path: string, method: string, headers: Record<string, string>, body?: string | Buffer | Uint8Array | Readable };

export declare class BodyReadable extends Readable {
  text (): Promise<string>
  json (): Promise<unknown>
  blob (): Promise<Blob>
  bytes (): Promise<Uint8Array>
  arrayBuffer (): Promise<ArrayBuffer>
}

export class DaemonSDK {
  private readonly baseUrl: string;
  protected readonly identifier: string;
  private readonly headers: Record<string, string>;
  private readonly connect: { ca: string, cert: string, key: string, servername: string };

  constructor(baseUrl: string, identifier: string, daemonRootKey: string, backendKey: string, headers: Record<string, string> = {}) {
    if (!baseUrl.endsWith("/")) {
      baseUrl += "/";
    }
    this.baseUrl = baseUrl;
    this.identifier = identifier;
    this.headers = headers;
    this.connect = {
      key: backendKey,
      cert: backendKey,
      ca: daemonRootKey,
      servername: `node-${identifier}`,
    };
  }

  createServerUrl(): URL {
    const daemonResultUrl = new URL(`${this.baseUrl}server`);
    return daemonResultUrl;
  }

  /** Creates a new server */
  async createServer(storage: string, resource: ServerResourcePayload, build: ServerBuildPayload, ports: Array<ServerPortEntryPayload>, network: ServerNetworkPayload): Promise<{ Ok : number } | { Err : CreateServerError }> {
    const daemonRequestUrl = this.createServerUrl();
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify({ storage: storage, resource: resource, build: build, ports: ports, network: network });
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as number };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/image_different":
            return { Err: errorId };
          case "server/image_missing_tag":
            return { Err: errorId };
          case "storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  updateServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}`);
    return daemonResultUrl;
  }

  /** Updates an existing server */
  async updateServer(serverId: number, resource: ServerResourcePayload | null, build: ServerBuildPayload | null, ports: Array<ServerPortEntryPayload> | null, network: ServerNetworkPayload | null): Promise<{ Ok : null } | { Err : UpdateServerError }> {
    const daemonRequestUrl = this.updateServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PATCH',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify({ resource: resource, build: build, ports: ports, network: network });
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "server/image_different":
            return { Err: errorId };
          case "server/image_missing_tag":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  deleteServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}`);
    return daemonResultUrl;
  }

  /** Deletes an existing server */
  async deleteServer(serverId: number): Promise<{ Ok : null } | { Err : DeleteServerError }> {
    const daemonRequestUrl = this.deleteServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "server/mutex_locked":
            return { Err: errorId };
          case "docker/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverStatusUrl(serverId: number, precise: boolean | null): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}`);
    if (precise != null) { daemonResultUrl.searchParams.append('precise', String(precise)); }
    return daemonResultUrl;
  }

  /** Gets the status of an existing server */
  async serverStatus(serverId: number, precise: boolean): Promise<{ Ok : PublicServerState } | { Err : ServerStatusError }> {
    const daemonRequestUrl = this.serverStatusUrl(serverId, precise);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as PublicServerState };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "server/mutex_locked":
            return { Err: errorId };
          case "docker/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  startServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/start`);
    return daemonResultUrl;
  }

  /** Starts an existing server */
  async startServer(serverId: number): Promise<{ Ok : null } | { Err : StartServerError }> {
    const daemonRequestUrl = this.startServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  stopServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/stop`);
    return daemonResultUrl;
  }

  /** Stops an existing server */
  async stopServer(serverId: number): Promise<{ Ok : null } | { Err : StopServerError }> {
    const daemonRequestUrl = this.stopServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  killServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/kill`);
    return daemonResultUrl;
  }

  /** Kills an existing server */
  async killServer(serverId: number): Promise<{ Ok : null } | { Err : KillServerError }> {
    const daemonRequestUrl = this.killServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  restartServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/restart`);
    return daemonResultUrl;
  }

  /** Restarts an existing server */
  async restartServer(serverId: number): Promise<{ Ok : null } | { Err : RestartServerError }> {
    const daemonRequestUrl = this.restartServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  execServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/exec`);
    return daemonResultUrl;
  }

  /** Executes a command inside an running server */
  async execServer(serverId: number, cmd: Array<string>): Promise<{ Ok : null } | { Err : ExecServerError }> {
    const daemonRequestUrl = this.execServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'POST',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(cmd);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "server/mutex_locked":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  wipeServerUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}server/${serverIdEncoded}/wipe`);
    return daemonResultUrl;
  }

  /** Deletes all files in an existing server */
  async wipeServer(serverId: number): Promise<{ Ok : null } | { Err : WipeServerError }> {
    const daemonRequestUrl = this.wipeServerUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverTerminalUrl(serverId: number, cols: number | null, rows: number | null, write: boolean, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/server/${serverIdEncoded}/terminal`);
    if (cols != null) { daemonResultUrl.searchParams.append('cols', String(cols)); }
    if (rows != null) { daemonResultUrl.searchParams.append('rows', String(rows)); }
    daemonResultUrl.searchParams.append('write', String(write));
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  serverStatsUrl(serverId: number, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/server/${serverIdEncoded}/stats`);
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  execServerInteractiveUrl(serverId: number, cols: number | null, rows: number | null, command: Array<string>, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/server/${serverIdEncoded}/exec`);
    if (cols != null) { daemonResultUrl.searchParams.append('cols', String(cols)); }
    if (rows != null) { daemonResultUrl.searchParams.append('rows', String(rows)); }
    daemonResultUrl.searchParams.append('command', String(command));
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  serverListBackupsUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}`);
    return daemonResultUrl;
  }

  /** Lists all backups of an existing server */
  async serverListBackups(serverId: number): Promise<{ Ok : Array<BackupResponseEntry> } | { Err : ServerListBackupsError }> {
    const daemonRequestUrl = this.serverListBackupsUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as Array<BackupResponseEntry> };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverCreateBackupOrderUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}`);
    return daemonResultUrl;
  }

  /** Creates a new backup (selecting the backup storage by order) */
  async serverCreateBackupOrder(serverId: number): Promise<{ Ok : bigint } | { Err : ServerCreateBackupOrderError }> {
    const daemonRequestUrl = this.serverCreateBackupOrderUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as bigint };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup_storage/no_suitable_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverCreateBackupUrl(serverId: number, backupStorage: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/${backupStorageEncoded}`);
    return daemonResultUrl;
  }

  /** Creates a new backup */
  async serverCreateBackup(serverId: number, backupStorage: string): Promise<{ Ok : bigint } | { Err : ServerCreateBackupError }> {
    const daemonRequestUrl = this.serverCreateBackupUrl(serverId, backupStorage);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as bigint };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup_storage/incompatible":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadBackupUrl(serverId: number, backupStorage: string, backupId: bigint): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const backupIdEncoded = encodeURIComponent(String(backupId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/${backupStorageEncoded}/${backupIdEncoded}`);
    return daemonResultUrl;
  }

  /** Downloads a backup */
  async serverDownloadBackup(serverId: number, backupStorage: string, backupId: bigint): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadBackupError }> {
    const daemonRequestUrl = this.serverDownloadBackupUrl(serverId, backupStorage, backupId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadBackupUserUrl(serverId: number, backupStorage: string, backupId: bigint, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const backupIdEncoded = encodeURIComponent(String(backupId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/backup/${serverIdEncoded}/${backupStorageEncoded}/${backupIdEncoded}`);
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  /** Downloads a backup (called by the user) */
  async serverDownloadBackupUser(serverId: number, backupStorage: string, backupId: bigint, timestamp: bigint, verify: UserSignature): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadBackupUserError }> {
    const daemonRequestUrl = this.serverDownloadBackupUserUrl(serverId, backupStorage, backupId, timestamp, verify);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/signature_invalid":
            return { Err: errorId };
          case "auth/signature_expired":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadRealtimeUrl(serverId: number, compression: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/rt`);
    daemonResultUrl.searchParams.append('compression', String(compression));
    return daemonResultUrl;
  }

  /** Downloads the contents of a server in realtime (without creating a backup) */
  async serverDownloadRealtime(serverId: number, compression: string): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadRealtimeError }> {
    const daemonRequestUrl = this.serverDownloadRealtimeUrl(serverId, compression);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "compression/unsupported":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadRealtimeUserUrl(serverId: number, timestamp: bigint, verify: UserSignature, compression: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/backup/${serverIdEncoded}/rt`);
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    daemonResultUrl.searchParams.append('compression', String(compression));
    return daemonResultUrl;
  }

  /** Downloads the contents of a server in realtime (without creating a backup and called by the user) */
  async serverDownloadRealtimeUser(serverId: number, timestamp: bigint, verify: UserSignature, compression: string): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadRealtimeUserError }> {
    const daemonRequestUrl = this.serverDownloadRealtimeUserUrl(serverId, timestamp, verify, compression);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/signature_invalid":
            return { Err: errorId };
          case "auth/signature_expired":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "compression/unsupported":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverSetBackupNameUrl(serverId: number, backupStorage: string, backupId: bigint): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const backupIdEncoded = encodeURIComponent(String(backupId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/${backupStorageEncoded}/${backupIdEncoded}`);
    return daemonResultUrl;
  }

  /** Sets the name of a backup */
  async serverSetBackupName(serverId: number, backupStorage: string, backupId: bigint, name: string | null): Promise<{ Ok : null } | { Err : ServerSetBackupNameError }> {
    const daemonRequestUrl = this.serverSetBackupNameUrl(serverId, backupStorage, backupId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PATCH',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(name);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup/not_found":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDeleteBackupUrl(serverId: number, backupStorage: string, backupId: bigint): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const backupIdEncoded = encodeURIComponent(String(backupId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/${backupStorageEncoded}/${backupIdEncoded}`);
    return daemonResultUrl;
  }

  /** Deletes a backup */
  async serverDeleteBackup(serverId: number, backupStorage: string, backupId: bigint): Promise<{ Ok : null } | { Err : ServerDeleteBackupError }> {
    const daemonRequestUrl = this.serverDeleteBackupUrl(serverId, backupStorage, backupId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "backup/not_found":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverRestoreBackupUrl(serverId: number, backupStorage: string, backupId: bigint): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const backupStorageEncoded = encodeURIComponent(String(backupStorage));
    const backupIdEncoded = encodeURIComponent(String(backupId));
    const daemonResultUrl = new URL(`${this.baseUrl}backup/${serverIdEncoded}/${backupStorageEncoded}/${backupIdEncoded}/restore`);
    return daemonResultUrl;
  }

  /** Restores a backup */
  async serverRestoreBackup(serverId: number, backupStorage: string, backupId: bigint): Promise<{ Ok : null } | { Err : ServerRestoreBackupError }> {
    const daemonRequestUrl = this.serverRestoreBackupUrl(serverId, backupStorage, backupId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "backup/not_found":
            return { Err: errorId };
          case "backup_storage/location_not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverListFilesUrl(serverId: number, path: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/list`);
    daemonResultUrl.searchParams.append('path', String(path));
    return daemonResultUrl;
  }

  /** List files inside a directory */
  async serverListFiles(serverId: number, path: string): Promise<{ Ok : Array<FileEntry> } | { Err : ServerListFilesError }> {
    const daemonRequestUrl = this.serverListFilesUrl(serverId, path);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as Array<FileEntry> };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/not_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverStatFileUrl(serverId: number, path: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/stat`);
    daemonResultUrl.searchParams.append('path', String(path));
    return daemonResultUrl;
  }

  /** Gets the info about a file */
  async serverStatFile(serverId: number, path: string): Promise<{ Ok : FileEntry } | { Err : ServerStatFileError }> {
    const daemonRequestUrl = this.serverStatFileUrl(serverId, path);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as FileEntry };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverRenameFileUrl(serverId: number, from: string, to: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/rename`);
    daemonResultUrl.searchParams.append('from', String(from));
    daemonResultUrl.searchParams.append('to', String(to));
    return daemonResultUrl;
  }

  /** Renames/Moves a file */
  async serverRenameFile(serverId: number, from: string, to: string): Promise<{ Ok : null } | { Err : ServerRenameFileError }> {
    const daemonRequestUrl = this.serverRenameFileUrl(serverId, from, to);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/already_exists":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverCopyFileUrl(serverId: number, from: string, to: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/copy`);
    daemonResultUrl.searchParams.append('from', String(from));
    daemonResultUrl.searchParams.append('to', String(to));
    return daemonResultUrl;
  }

  /** Copies a file */
  async serverCopyFile(serverId: number, from: string, to: string): Promise<{ Ok : null } | { Err : ServerCopyFileError }> {
    const daemonRequestUrl = this.serverCopyFileUrl(serverId, from, to);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadFileUrl(serverId: number, path: string, limit: bigint | null): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/file`);
    daemonResultUrl.searchParams.append('path', String(path));
    if (limit != null) { daemonResultUrl.searchParams.append('limit', String(limit)); }
    return daemonResultUrl;
  }

  /** Downloads a file */
  async serverDownloadFile(serverId: number, path: string, limit: bigint): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadFileError }> {
    const daemonRequestUrl = this.serverDownloadFileUrl(serverId, path, limit);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "file/limit_reached":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDownloadFileUserUrl(serverId: number, path: string, limit: bigint | null, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/files/${serverIdEncoded}/file`);
    daemonResultUrl.searchParams.append('path', String(path));
    if (limit != null) { daemonResultUrl.searchParams.append('limit', String(limit)); }
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  /** Downloads a file (called by the user) */
  async serverDownloadFileUser(serverId: number, path: string, limit: bigint, timestamp: bigint, verify: UserSignature): Promise<{ Ok : BodyReadable } | { Err : ServerDownloadFileUserError }> {
    const daemonRequestUrl = this.serverDownloadFileUserUrl(serverId, path, limit, timestamp, verify);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: response.body };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/signature_invalid":
            return { Err: errorId };
          case "auth/signature_expired":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "file/limit_reached":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverUploadFileUrl(serverId: number, path: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/file`);
    daemonResultUrl.searchParams.append('path', String(path));
    return daemonResultUrl;
  }

  /** Uploads a file */
  async serverUploadFile(serverId: number, path: string, length: string | null, data: string | Buffer | Uint8Array | Readable): Promise<{ Ok : null } | { Err : ServerUploadFileError }> {
    const daemonRequestUrl = this.serverUploadFileUrl(serverId, path);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    if (length != null) { daemonRequest.headers["content-length"] = length; }
    daemonRequest.body = data;
    daemonRequest.headers['content-type'] = 'application/octet-stream';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverUploadFileUserUrl(serverId: number, path: string, timestamp: bigint, verify: UserSignature): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}user/files/${serverIdEncoded}/file`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('timestamp', String(timestamp));
    daemonResultUrl.searchParams.append('verify', String(verify));
    return daemonResultUrl;
  }

  /** Uploads a file (called by the user) */
  async serverUploadFileUser(serverId: number, path: string, timestamp: bigint, verify: UserSignature, length: string | null, data: string | Buffer | Uint8Array | Readable): Promise<{ Ok : null } | { Err : ServerUploadFileUserError }> {
    const daemonRequestUrl = this.serverUploadFileUserUrl(serverId, path, timestamp, verify);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    if (length != null) { daemonRequest.headers["content-length"] = length; }
    daemonRequest.body = data;
    daemonRequest.headers['content-type'] = 'application/octet-stream';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/signature_invalid":
            return { Err: errorId };
          case "auth/signature_expired":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverWriteFileByUrlUrl(serverId: number, path: string, url: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/url`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('url', String(url));
    return daemonResultUrl;
  }

  /** Writes a file using a url */
  async serverWriteFileByUrl(serverId: number, path: string, url: string): Promise<{ Ok : null } | { Err : ServerWriteFileByUrlError }> {
    const daemonRequestUrl = this.serverWriteFileByUrlUrl(serverId, path, url);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };
          case "http_request/failed":
            return { Err: errorId };
          case "http_request/not_success":
            return { Err: errorId };
          case "http_request/read_failed":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverWriteFileByTemplateUrl(serverId: number, path: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/template`);
    daemonResultUrl.searchParams.append('path', String(path));
    return daemonResultUrl;
  }

  /** Writes a file using a template */
  async serverWriteFileByTemplate(serverId: number, path: string, template: ServerFileWrite): Promise<{ Ok : null } | { Err : ServerWriteFileByTemplateError }> {
    const daemonRequestUrl = this.serverWriteFileByTemplateUrl(serverId, path);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'POST',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(template);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "storage/patch_failed":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverDeleteFileUrl(serverId: number, path: string, recursive: boolean): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/file`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('recursive', String(recursive));
    return daemonResultUrl;
  }

  /** Deletes a file */
  async serverDeleteFile(serverId: number, path: string, recursive: boolean): Promise<{ Ok : null } | { Err : ServerDeleteFileError }> {
    const daemonRequestUrl = this.serverDeleteFileUrl(serverId, path, recursive);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/not_directory":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverCreateDirectoryUrl(serverId: number, path: string, recursive: boolean): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/mkdir`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('recursive', String(recursive));
    return daemonResultUrl;
  }

  /** Creates a new directory */
  async serverCreateDirectory(serverId: number, path: string, recursive: boolean): Promise<{ Ok : null } | { Err : ServerCreateDirectoryError }> {
    const daemonRequestUrl = this.serverCreateDirectoryUrl(serverId, path, recursive);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/already_exists":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverSymlinkUrl(serverId: number, path: string, content: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/symlink`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('content', String(content));
    return daemonResultUrl;
  }

  /** Creates a new symlink */
  async serverSymlink(serverId: number, path: string, content: string): Promise<{ Ok : null } | { Err : ServerSymlinkError }> {
    const daemonRequestUrl = this.serverSymlinkUrl(serverId, path, content);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/already_exists":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverUnpackArchiveUrl(serverId: number, path: string, dest: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/archive/unpack`);
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('dest', String(dest));
    return daemonResultUrl;
  }

  /** Unpacks an existing archive */
  async serverUnpackArchive(serverId: number, path: string, dest: string): Promise<{ Ok : null } | { Err : ServerUnpackArchiveError }> {
    const daemonRequestUrl = this.serverUnpackArchiveUrl(serverId, path, dest);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "compression/unsupported":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  serverCreateArchiveUrl(serverId: number, src: string, path: string, compression: string): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}files/${serverIdEncoded}/archive/create`);
    daemonResultUrl.searchParams.append('src', String(src));
    daemonResultUrl.searchParams.append('path', String(path));
    daemonResultUrl.searchParams.append('compression', String(compression));
    return daemonResultUrl;
  }

  /** Creates an archive */
  async serverCreateArchive(serverId: number, src: string, path: string, compression: string): Promise<{ Ok : null } | { Err : ServerCreateArchiveError }> {
    const daemonRequestUrl = this.serverCreateArchiveUrl(serverId, src, path, compression);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "storage/not_found":
            return { Err: errorId };
          case "storage/mutex_locked":
            return { Err: errorId };
          case "storage/failure":
            return { Err: errorId };
          case "storage/quota_reached":
            return { Err: errorId };
          case "file/not_found":
            return { Err: errorId };
          case "file/sandbox_violated":
            return { Err: errorId };
          case "file/is_directory":
            return { Err: errorId };
          case "compression/unsupported":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  createNetworkUrl(): URL {
    const daemonResultUrl = new URL(`${this.baseUrl}network`);
    return daemonResultUrl;
  }

  /** Creates a new network */
  async createNetwork(ipv4: number, ipv6: number): Promise<{ Ok : number } | { Err : CreateNetworkError }> {
    const daemonRequestUrl = this.createNetworkUrl();
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify({ ipv4: ipv4, ipv6: ipv6 });
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as number };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "network/prefix_collision":
            return { Err: errorId };
          case "network/prefix_invalid":
            return { Err: errorId };
          case "docker/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  deleteNetworkUrl(networkId: number): URL {
    const networkIdEncoded = encodeURIComponent(String(networkId));
    const daemonResultUrl = new URL(`${this.baseUrl}network/${networkIdEncoded}`);
    return daemonResultUrl;
  }

  /** Deletes an existing network */
  async deleteNetwork(networkId: number): Promise<{ Ok : null } | { Err : DeleteNetworkError }> {
    const daemonRequestUrl = this.deleteNetworkUrl(networkId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "network/not_found":
            return { Err: errorId };
          case "docker/failure":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  resourceInfoUrl(): URL {
    const daemonResultUrl = new URL(`${this.baseUrl}resource`);
    return daemonResultUrl;
  }

  /** Gets resource information */
  async resourceInfo(): Promise<{ Ok : ResourceResponse } | { Err : ResourceInfoError }> {
    const daemonRequestUrl = this.resourceInfoUrl();
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'GET',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as ResourceResponse };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };
          case "ResourceError":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  createDatabaseUrl(): URL {
    const daemonResultUrl = new URL(`${this.baseUrl}database`);
    return daemonResultUrl;
  }

  /** Creates a new database */
  async createDatabase(nameSuffix: string): Promise<{ Ok : number } | { Err : CreateDatabaseError }> {
    const daemonRequestUrl = this.createDatabaseUrl();
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(nameSuffix);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as number };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "database/invalid_identifier":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  updateDatabaseUrl(databaseId: number): URL {
    const databaseIdEncoded = encodeURIComponent(String(databaseId));
    const daemonResultUrl = new URL(`${this.baseUrl}database/${databaseIdEncoded}`);
    return daemonResultUrl;
  }

  /** Updates an existing database */
  async updateDatabase(databaseId: number, users: { [key in string]?: DatabaseUserPayload }, passwordless: { [key in number]?: DatabaseUserPermissions }): Promise<{ Ok : null } | { Err : UpdateDatabaseError }> {
    const daemonRequestUrl = this.updateDatabaseUrl(databaseId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PATCH',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify({ users: users, passwordless: passwordless });
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "database/not_found":
            return { Err: errorId };
          case "database/invalid_identifier":
            return { Err: errorId };
          case "database/invalid_hash":
            return { Err: errorId };
          case "database/permissionless_entry":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  deleteDatabaseUrl(databaseId: number): URL {
    const databaseIdEncoded = encodeURIComponent(String(databaseId));
    const daemonResultUrl = new URL(`${this.baseUrl}database/${databaseIdEncoded}`);
    return daemonResultUrl;
  }

  /** Updates an existing database */
  async deleteDatabase(databaseId: number): Promise<{ Ok : null } | { Err : DeleteDatabaseError }> {
    const daemonRequestUrl = this.deleteDatabaseUrl(databaseId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "database/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  createScheduleUrl(serverId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const daemonResultUrl = new URL(`${this.baseUrl}schedule/${serverIdEncoded}`);
    return daemonResultUrl;
  }

  /** Creates a new schedule */
  async createSchedule(serverId: number, payload: SchedulePayload): Promise<{ Ok : number } | { Err : CreateScheduleError }> {
    const daemonRequestUrl = this.createScheduleUrl(serverId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PUT',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(payload);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: await response.body.json() as number };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  updateScheduleUrl(serverId: number, scheduleId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const scheduleIdEncoded = encodeURIComponent(String(scheduleId));
    const daemonResultUrl = new URL(`${this.baseUrl}schedule/${serverIdEncoded}/${scheduleIdEncoded}`);
    return daemonResultUrl;
  }

  /** Updates a schedule */
  async updateSchedule(serverId: number, scheduleId: number, payload: SchedulePayload): Promise<{ Ok : null } | { Err : UpdateScheduleError }> {
    const daemonRequestUrl = this.updateScheduleUrl(serverId, scheduleId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'PATCH',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });
    daemonRequest.body = JSON.stringify(payload);
    daemonRequest.headers['content-type'] = 'application/json';

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "schedule/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }

  deleteScheduleUrl(serverId: number, scheduleId: number): URL {
    const serverIdEncoded = encodeURIComponent(String(serverId));
    const scheduleIdEncoded = encodeURIComponent(String(scheduleId));
    const daemonResultUrl = new URL(`${this.baseUrl}schedule/${serverIdEncoded}/${scheduleIdEncoded}`);
    return daemonResultUrl;
  }

  /** Deletes a schedule */
  async deleteSchedule(serverId: number, scheduleId: number): Promise<{ Ok : null } | { Err : DeleteScheduleError }> {
    const daemonRequestUrl = this.deleteScheduleUrl(serverId, scheduleId);
    const daemonRequest: DaemonSDKRequest = {
      path: `${daemonRequestUrl.pathname}${daemonRequestUrl.search}`,
      method: 'DELETE',
      headers: Object.assign({}, this.headers),
    };
    const client = new UndiciClient(daemonRequestUrl.origin, {
      connect: this.connect,
      bodyTimeout: 0,
      headersTimeout: 0,
    });

    try {
      const response = await client.request(daemonRequest);
      if (response.statusCode >= 200 && response.statusCode <= 299) {
        return { Ok: null };
      } else {
        const errorResponse = await response.body.json() as { error: { id: string, msg: string } };
        const errorId = errorResponse.error.id;
        switch (errorId) {
          case "auth/invalid":
            return { Err: errorId };
          case "server/not_found":
            return { Err: errorId };
          case "sql/failure":
            return { Err: errorId };

          default:
            throw new Error(`Unknown error: ${errorId}`);
        }
      }

    } catch (err) {
      return { Err: { "Communication": err } };
    }
                
  }



}

export class DaemonSDKWithUserKey extends DaemonSDK {
  private readonly userKey: string | Uint8Array;

  constructor(baseUrl: string, identifier: string, daemonRootKey: string, backendKey: string, headers: Record<string, string> = {}) {
    super(baseUrl, identifier, daemonRootKey, backendKey, headers);

    let userKey: forge.util.ByteStringBuffer | null = null;
    const daemonRootKeyDecoded = forge.pem.decode(daemonRootKey);
    for (const entry of daemonRootKeyDecoded) {
      if (entry.type === "PRIVATE USER KEY") {
        userKey = forge.util.createBuffer(entry.body);
        break;
      }
    }
    if (userKey === null) {
      throw new Error("User Key is not present");
    }
    const keyAsn1 = forge.asn1.fromDer(userKey as forge.util.ByteStringBuffer);
    const userPrivateKey = forge.pki.ed25519.privateKeyFromAsn1(keyAsn1);

    this.userKey = userPrivateKey.privateKeyBytes;
  }

  async serverTerminalUrlSigned(serverId: number, cols: number | null, rows: number | null, write: boolean): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newTerminalAsync(this.userKey, timestamp, this.identifier, serverId, write);
    return this.serverTerminalUrl(serverId, cols, rows, write, timestamp, signature);
  }

  async serverStatsUrlSigned(serverId: number): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newStatsAsync(this.userKey, timestamp, this.identifier, serverId);
    return this.serverStatsUrl(serverId, timestamp, signature);
  }

  async execServerInteractiveUrlSigned(serverId: number, cols: number | null, rows: number | null, command: Array<string>): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newExecAsync(this.userKey, timestamp, this.identifier, serverId, command);
    return this.execServerInteractiveUrl(serverId, cols, rows, command, timestamp, signature);
  }

  async serverDownloadBackupUserUrlSigned(serverId: number, backupStorage: string, backupId: bigint): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newServerBackupDownloadAsync(this.userKey, timestamp, this.identifier, serverId, backupStorage, backupId);
    return this.serverDownloadBackupUserUrl(serverId, backupStorage, backupId, timestamp, signature);
  }

  async serverDownloadRealtimeUserUrlSigned(serverId: number, compression: string): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newServerRealtimeDownloadAsync(this.userKey, timestamp, this.identifier, serverId, compression);
    return this.serverDownloadRealtimeUserUrl(serverId, timestamp, signature, compression);
  }

  async serverDownloadFileUserUrlSigned(serverId: number, path: string, limit: bigint | null): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newServerFileDownloadAsync(this.userKey, timestamp, this.identifier, serverId, path);
    return this.serverDownloadFileUserUrl(serverId, path, limit, timestamp, signature);
  }

  async serverUploadFileUserUrlSigned(serverId: number, path: string): Promise<URL> {
    const timestamp: bigint = BigInt(Date.now());
    const signature = await UserSignature.newServerFileUploadAsync(this.userKey, timestamp, this.identifier, serverId, path);
    return this.serverUploadFileUserUrl(serverId, path, timestamp, signature);
  }



}

export const DAEMON_ROOT_KEY_COMMON_NAME: string = "Novium CA";
export const DAEMON_BACKEND_KEY_COMMON_NAME: string = "Novium Backend";


export function generateDaemonRootKey(): string {
  // Ed25519 is not supported with x509 in node-forge.
  const keypair = forge.pki.rsa.generateKeyPair(2048);

  const cert = forge.pki.createCertificate();
  cert.publicKey = keypair.publicKey;
  cert.validity.notBefore = new Date();
  cert.validity.notAfter = new Date();
  cert.validity.notAfter.setFullYear(cert.validity.notBefore.getFullYear() + 64);

  const attrs = [{
    name: 'commonName',
    value: DAEMON_ROOT_KEY_COMMON_NAME,
  }];

  cert.setSubject(attrs);
  cert.setIssuer(attrs);
  cert.setExtensions([
    {
      name: 'basicConstraints',
      cA: true, // It was this weirdly spelled in the example, see https://github.com/digitalbazaar/forge/blob/main/examples/create-cert.js
    },
    {
      name: 'keyUsage',
      keyCertSign: true,
      digitalSignature: true,
      cRLSign: true,
    },
    {
      name: 'subjectKeyIdentifier'
    },
  ]);

  cert.sign(keypair.privateKey);

  const { privateKey: privateUserKey } = crypto.generateKeyPairSync('ed25519', {
    publicKeyEncoding: {
      type: 'spki',
      format: 'pem',
    },
    privateKeyEncoding: {
      type: 'pkcs8',
      format: 'pem',
    },
  });

  return `\n${forge.pki.privateKeyToPem(keypair.privateKey)}\n${forge.pki.certificateToPem(cert)}\n${privateUserKey.toString().replaceAll("PRIVATE KEY", "PRIVATE USER KEY")}\n`;
}

export function privateDaemonRootKeyToPublic(privateKey: string): string {
  const publicKey = new crypto.X509Certificate(privateKey);

  let userKey: string | null = null;

  const daemonRootKeyDecoded = forge.pem.decode(privateKey);
  for (const entry of daemonRootKeyDecoded) {
    if (entry.type === "PRIVATE USER KEY") {
      entry.type = "PRIVATE KEY";
      userKey = forge.pem.encode(entry);
      break;
    }
  }
  if (userKey !== null) {
    userKey = crypto.createPublicKey(userKey).export({
      type: 'spki',
      format: 'pem',
    }).toString().replaceAll("PUBLIC KEY", "USER KEY");
  }

  return `\n${publicKey}\n${userKey === null ? "" : `${userKey}\n`}`;
}

export function generateBackendKey(daemonRootKey: string): string {
  const privateDaemonRootKey = forge.pki.privateKeyFromPem(daemonRootKey);
  const daemonRootKeyCert = forge.pki.certificateFromPem(privateDaemonRootKeyToPublic(daemonRootKey));

  // Ed25519 is not supported with x509 in node-forge.
  const keypair = forge.pki.rsa.generateKeyPair(2048);

  const cert = forge.pki.createCertificate();
  cert.publicKey = keypair.publicKey;
  cert.validity.notBefore = new Date();
  cert.validity.notAfter = new Date();
  cert.validity.notAfter.setFullYear(cert.validity.notBefore.getFullYear() + 64);

  const attrs = [{
    name: 'commonName',
    value: DAEMON_BACKEND_KEY_COMMON_NAME,
  }];

  cert.setSubject(attrs);
  cert.setIssuer(daemonRootKeyCert.subject.attributes);
  cert.setExtensions([
    {
      name: 'keyUsage',
      digitalSignature: true,
    },
    {
      name: 'extKeyUsage',
      clientAuth: true,
    },
    {
      name: 'subjectKeyIdentifier'
    },
    {
      name: 'authorityKeyIdentifier',
    }
  ]);

  cert.sign(privateDaemonRootKey);

  return `\n${forge.pki.privateKeyToPem(keypair.privateKey)}\n${forge.pki.certificateToPem(cert)}\n`;
}

export function generateDaemonKey(daemonRootKey: string, csr: Uint8Array, identification: string): string {
  const privateDaemonRootKey = forge.pki.privateKeyFromPem(daemonRootKey);
  const daemonRootKeyCert = forge.pki.certificateFromPem(privateDaemonRootKeyToPublic(daemonRootKey));

  const csrAsn1 = forge.asn1.fromDer(new forge.util.ByteStringBuffer(csr));

  const csrParsed = forge.pki.certificationRequestFromAsn1(csrAsn1);
  if (!csrParsed.verify()) {
    throw new Error("CSR Signature doesn't match");
  }
  const cn = csrParsed.subject.getField('CN');
  if (cn === null) {
    throw new Error("Common Name is missing");
  }
  if (cn.value !== 'node') {
    throw new Error("Common Name isn't node");
  }
  if (csrParsed.publicKey === null) {
    throw new Error("Public Key is null");
  }

  const cert = forge.pki.createCertificate();
  cert.publicKey = csrParsed.publicKey;
  cert.validity.notBefore = new Date();
  cert.validity.notAfter = new Date();
  cert.validity.notAfter.setFullYear(cert.validity.notBefore.getFullYear() + 64);

  const attrs = [{
    name: 'commonName',
    value: `node-${identification}`,
  }];

  cert.setSubject(attrs);
  cert.setIssuer(daemonRootKeyCert.subject.attributes);
  cert.setExtensions([
    {
      name: 'keyUsage',
      digitalSignature: true,
    },
    {
      name: 'extKeyUsage',
      serverAuth: true,
    },
    {
      name: 'subjectKeyIdentifier'
    },
    {
      name: 'authorityKeyIdentifier',
    },
    {
      name: 'subjectAltName',
      altNames: [{
        type: 2, // DNS
        value: `node-${identification}`
      }],
    }
  ]);

  cert.sign(privateDaemonRootKey);

  return `\n${forge.pki.certificateToPem(cert)}\n`;
}

export function tempDaemonKeyToPem(der: Uint8Array): string {
  return forge.pem.encode({
    type: 'CERTIFICATE',
    body: Buffer.from(der).toString()
  });
}