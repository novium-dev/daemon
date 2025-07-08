
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

/** {user_methods} */

}