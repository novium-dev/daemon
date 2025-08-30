/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */

import * as ed from '@noble/ed25519';
import { Client as UndiciClient } from 'undici';
import * as crypto from 'node:crypto';
import { Readable } from 'node:stream';
import * as forge from 'node-forge';

/** {types} */

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

/** {sdk_implementation} */

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

/** {sdk_user_implementation} */

}

/** {sdk_config_consts} */

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