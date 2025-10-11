import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  getDataSourcePda,
  createTestDataSourceInfo,
} from "../setup";

describe("Update Data Source Instruction", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  // Helper function to create a data source for testing updates
  async function createTestDataSource(
    name: string,
    dataSourceType: number = 1
  ) {
    const dataSourceInfo = createTestDataSourceInfo(dataSourceType, name);

    const [dataSourcePDA] = getDataSourcePda(
      ctx.molphaProgram.programId,
      ctx.authority.publicKey,
      name,
      dataSourceType
    );

    await ctx.molphaProgram.methods
      .createDataSource(dataSourceInfo as any)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    return dataSourcePDA;
  }

  it("Successfully updates data source metadata hash", async () => {
    // First create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 1");

    // Get the original data source account
    const originalDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    // Create a new metadata hash
    const newMetadataHash = new Uint8Array(32);
    newMetadataHash.fill(42); // Different from the original (which was filled with 1s)

    // Update the data source
    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(newMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    // Fetch the updated data source account
    const updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    // Verify the metadata hash was updated
    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(newMetadataHash)
    );

    // Verify other fields remain unchanged
    assert.equal(
      updatedDataSource.authority.toBase58(),
      originalDataSource.authority.toBase58()
    );
    assert.deepEqual(
      updatedDataSource.dataSourceType,
      originalDataSource.dataSourceType
    );
    assert.equal(updatedDataSource.name, originalDataSource.name);
    assert.equal(
      updatedDataSource.createdAt.toNumber(),
      originalDataSource.createdAt.toNumber()
    );
    assert.equal(updatedDataSource.bump, originalDataSource.bump);
  });

  it("Successfully updates metadata hash to all zeros", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 2");

    // Create a zero-filled metadata hash
    const zeroMetadataHash = new Uint8Array(32);
    zeroMetadataHash.fill(0);

    // Update the data source
    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(zeroMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    // Verify the update
    const updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(zeroMetadataHash)
    );
  });

  it("Successfully updates metadata hash to all 255s", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 3");

    // Create a max-filled metadata hash
    const maxMetadataHash = new Uint8Array(32);
    maxMetadataHash.fill(255);

    // Update the data source
    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(maxMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    // Verify the update
    const updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(maxMetadataHash)
    );
  });

  it("Fails to update data source with wrong authority", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 4");

    // Create a different authority
    const wrongAuthority = Keypair.generate();

    const newMetadataHash = new Uint8Array(32);
    newMetadataHash.fill(99);

    try {
      await ctx.molphaProgram.methods
        .updateDataSource(Array.from(newMetadataHash))
        .accountsPartial({
          authority: wrongAuthority.publicKey,
          dataSource: dataSourcePDA,
        })
        .signers([wrongAuthority])
        .rpc();
      assert.fail("Should have failed with wrong authority");
    } catch (error: any) {
      assert.ok(
        error.message.includes("has_one") ||
          error.message.includes("custom program error") ||
          error.message.includes("ConstraintHasOne") ||
          error.message.includes("A has_one constraint was violated") ||
          error.message.includes("ConstraintSeeds") ||
          error.message.includes("A seeds constraint was violated")
      );
    }
  });

  it("Fails to update non-existent data source", async () => {
    // Create a fake data source PDA that doesn't exist
    const fakeDataSourcePDA = Keypair.generate().publicKey;

    const newMetadataHash = new Uint8Array(32);
    newMetadataHash.fill(99);

    try {
      await ctx.molphaProgram.methods
        .updateDataSource(Array.from(newMetadataHash))
        .accountsPartial({
          authority: ctx.authority.publicKey,
          dataSource: fakeDataSourcePDA,
        })
        .rpc();
      assert.fail("Should have failed with non-existent data source");
    } catch (error: any) {
      assert.ok(
        error.message.includes("AccountNotInitialized") ||
          error.message.includes("custom program error") ||
          error.message.includes("Account does not exist")
      );
    }
  });

  it("Fails to update data source with wrong PDA derivation", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 5");

    // Create a completely different PDA that doesn't match the seeds
    const fakeDataSourcePDA = Keypair.generate().publicKey;

    const newMetadataHash = new Uint8Array(32);
    newMetadataHash.fill(99);

    // Try to update using a completely wrong PDA
    try {
      await ctx.molphaProgram.methods
        .updateDataSource(Array.from(newMetadataHash))
        .accountsPartial({
          authority: ctx.authority.publicKey,
          dataSource: fakeDataSourcePDA, // Completely wrong PDA
        })
        .rpc();
      assert.fail("Should have failed with wrong PDA derivation");
    } catch (error: any) {
      assert.ok(
        error.message.includes("has_one") ||
          error.message.includes("custom program error") ||
          error.message.includes("ConstraintHasOne") ||
          error.message.includes("A has_one constraint was violated") ||
          error.message.includes("ConstraintSeeds") ||
          error.message.includes("A seeds constraint was violated") ||
          error.message.includes("AccountNotInitialized")
      );
    }
  });

  it("Successfully updates same metadata hash (no-op)", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 6");

    // Get the original metadata hash
    const originalDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );
    const originalMetadataHash = Array.from(originalDataSource.metadataHash);

    // Update with the same metadata hash
    await ctx.molphaProgram.methods
      .updateDataSource(originalMetadataHash)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    // Verify the data source is still the same
    const updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      originalMetadataHash
    );
  });

  it("Successfully updates metadata hash multiple times", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 7");

    // First update
    const firstMetadataHash = new Uint8Array(32);
    firstMetadataHash.fill(10);

    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(firstMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    let updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );
    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(firstMetadataHash)
    );

    // Second update
    const secondMetadataHash = new Uint8Array(32);
    secondMetadataHash.fill(20);

    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(secondMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );
    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(secondMetadataHash)
    );

    // Third update
    const thirdMetadataHash = new Uint8Array(32);
    thirdMetadataHash.fill(30);

    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(thirdMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );
    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(thirdMetadataHash)
    );
  });

  it("Fails to update data source with invalid account owner", async () => {
    // Create a fake account that's not owned by the program
    const fakeAccount = Keypair.generate();

    const newMetadataHash = new Uint8Array(32);
    newMetadataHash.fill(99);

    try {
      await ctx.molphaProgram.methods
        .updateDataSource(Array.from(newMetadataHash))
        .accountsPartial({
          authority: ctx.authority.publicKey,
          dataSource: fakeAccount.publicKey, // Not a valid data source account
        })
        .rpc();
      assert.fail("Should have failed with invalid account owner");
    } catch (error: any) {
      assert.ok(
        error.message.includes("AccountNotInitialized") ||
          error.message.includes("custom program error") ||
          error.message.includes("Account does not exist")
      );
    }
  });

  it("Successfully updates metadata hash with complex pattern", async () => {
    // Create a data source
    const dataSourcePDA = await createTestDataSource("Test Data Source 8");

    // Create a complex metadata hash pattern
    const complexMetadataHash = new Uint8Array(32);
    for (let i = 0; i < 32; i++) {
      complexMetadataHash[i] = (i * 7 + 13) % 256; // Some pattern
    }

    // Update the data source
    await ctx.molphaProgram.methods
      .updateDataSource(Array.from(complexMetadataHash))
      .accountsPartial({
        authority: ctx.authority.publicKey,
        dataSource: dataSourcePDA,
      })
      .rpc();

    // Verify the update
    const updatedDataSource = await ctx.molphaProgram.account.dataSource.fetch(
      dataSourcePDA
    );

    assert.deepEqual(
      Array.from(updatedDataSource.metadataHash),
      Array.from(complexMetadataHash)
    );
  });
});
