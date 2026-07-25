// Stub implementations — real chain calls need @polkadot/api polyfills for React Native

export interface ZkRegistration {
  zkProof: Uint8Array;
  publicInputs: string[];
}

export async function isCitizen(_address: string): Promise<boolean> { return false; }
export async function registerCitizen(_reg: ZkRegistration): Promise<void> {}
export async function getSigningKeypair(): Promise<any> {
  return { keypair: { address: '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY' }, nullifierHash: '0x0000' };
}
