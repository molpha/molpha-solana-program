/**
 * Molpha Oracle Client
 *
 * Provides Pyth and Switchboard compatible interfaces for consuming Molpha feeds.
 * This keeps the core flexible while offering standard oracle interfaces.
 */

import { Connection, PublicKey } from "@solana/web3.js";
import { Program, AnchorProvider } from "@coral-xyz/anchor";
import { BN } from "bn.js";

/**
 * Pyth-compatible price data structure
 */
export interface PythPrice {
  price: BN; // Price as integer (multiply by 10^expo for actual value)
  conf: BN; // Confidence interval
  expo: number; // Price exponent
  publishTime: BN; // Unix timestamp
}

/**
 * Switchboard-compatible result structure
 */
export interface SwitchboardResult {
  result: number; // Decimal result
  numSuccess: number; // Number of successful oracle responses
  numError: number; // Number of error responses
  minResponse: number; // Minimum value from responses
  maxResponse: number; // Maximum value from responses
  stdDev: number; // Standard deviation
  timestamp: BN; // Unix timestamp
}

/**
 * Price status enum
 */
export enum PriceStatus {
  Unknown = "unknown",
  Trading = "trading",
  Halted = "halted",
}

/**
 * Generic oracle price interface
 */
export interface OraclePrice {
  value: number; // Decimal value
  timestamp: number; // Unix timestamp
  confidence?: number; // Optional confidence/uncertainty
  status: PriceStatus;
}

/**
 * Molpha Oracle Client with standard oracle compatibility
 */
export class MolphaOracleClient {
  constructor(private program: Program, private connection: Connection) {}

  /**
   * Get raw feed data
   */
  async getFeed(feedAddress: PublicKey) {
    return await this.program.account.feed.fetch(feedAddress);
  }

  /**
   * Get Pyth-compatible price data
   *
   * @param feedAddress - Address of the Molpha feed
   * @returns PythPrice structure
   */
  async getPythPrice(feedAddress: PublicKey): Promise<PythPrice> {
    const feed = await this.getFeed(feedAddress);
    const answer = feed.latestAnswer;

    // Extract price data from Answer.value (first 32 bytes)
    // Layout: [price: i64][conf: u64][expo: i32][reserved: 12 bytes]
    const value = Buffer.from(answer.value);

    const price = value.readBigInt64LE(0);
    const conf = value.readBigUInt64LE(8);
    const expo = value.readInt32LE(16);

    return {
      price: new BN(price.toString()),
      conf: new BN(conf.toString()),
      expo,
      publishTime: new BN(answer.timestamp.toString()),
    };
  }

  /**
   * Get Pyth price with staleness check
   *
   * @param feedAddress - Address of the Molpha feed
   * @param maxAge - Maximum age in seconds
   * @returns PythPrice structure
   * @throws Error if price is too old
   */
  async getPythPriceNoOlderThan(
    feedAddress: PublicKey,
    maxAge: number
  ): Promise<PythPrice> {
    const price = await this.getPythPrice(feedAddress);
    const now = Math.floor(Date.now() / 1000);
    const age = now - price.publishTime.toNumber();

    if (age > maxAge) {
      throw new Error(`Price is too old: ${age}s > ${maxAge}s`);
    }

    return price;
  }

  /**
   * Get Switchboard-compatible result data
   *
   * @param feedAddress - Address of the Molpha feed
   * @param decimals - Number of decimals (default: 8)
   * @returns SwitchboardResult structure
   */
  async getSwitchboardResult(
    feedAddress: PublicKey,
    decimals: number = 8
  ): Promise<SwitchboardResult> {
    const feed = await this.getFeed(feedAddress);
    const answer = feed.latestAnswer;

    // Extract aggregator data from Answer.value
    // Layout: [value: i64][std_dev: i64][min: i64][max: i64]
    const value = Buffer.from(answer.value);

    const result = value.readBigInt64LE(0);
    const stdDev = value.readBigInt64LE(8);
    const min = value.readBigInt64LE(16);
    const max = value.readBigInt64LE(24);

    const divisor = Math.pow(10, decimals);

    return {
      result: Number(result) / divisor,
      numSuccess: 1, // Single answer
      numError: 0,
      minResponse: Number(min) / divisor,
      maxResponse: Number(max) / divisor,
      stdDev: Number(stdDev) / divisor,
      timestamp: new BN(answer.timestamp.toString()),
    };
  }

  /**
   * Get simple decimal price value (generic interface)
   *
   * @param feedAddress - Address of the Molpha feed
   * @param decimals - Number of decimals (default: 8)
   * @returns OraclePrice structure with decimal value
   */
  async getPrice(
    feedAddress: PublicKey,
    decimals: number = 8
  ): Promise<OraclePrice> {
    const feed = await this.getFeed(feedAddress);
    const pythPrice = await this.getPythPrice(feedAddress);

    const price = Number(pythPrice.price) * Math.pow(10, pythPrice.expo);
    const confidence = Number(pythPrice.conf) * Math.pow(10, pythPrice.expo);

    // Determine status
    const now = Math.floor(Date.now() / 1000);
    const isActive = feed.subscriptionDueTime.toNumber() > now;
    const age = now - pythPrice.publishTime.toNumber();
    const expectedFreshness = feed.frequency.toNumber() * 2;

    let status: PriceStatus;
    if (!isActive) {
      status = PriceStatus.Halted;
    } else if (age <= expectedFreshness) {
      status = PriceStatus.Trading;
    } else {
      status = PriceStatus.Unknown;
    }

    return {
      value: price,
      timestamp: pythPrice.publishTime.toNumber(),
      confidence,
      status,
    };
  }

  /**
   * Get price with staleness check
   */
  async getPriceNoOlderThan(
    feedAddress: PublicKey,
    maxAge: number,
    decimals: number = 8
  ): Promise<OraclePrice> {
    const price = await this.getPrice(feedAddress, decimals);
    const now = Math.floor(Date.now() / 1000);
    const age = now - price.timestamp;

    if (age > maxAge) {
      throw new Error(`Price is too old: ${age}s > ${maxAge}s`);
    }

    return price;
  }

  /**
   * Check if feed subscription is active
   */
  async isSubscriptionActive(feedAddress: PublicKey): Promise<boolean> {
    const feed = await this.getFeed(feedAddress);
    const now = Math.floor(Date.now() / 1000);
    return feed.subscriptionDueTime.toNumber() > now;
  }

  /**
   * Get feed price history (last N answers)
   */
  async getPriceHistory(
    feedAddress: PublicKey,
    decimals: number = 8
  ): Promise<OraclePrice[]> {
    const feed = await this.getFeed(feedAddress);

    return feed.answerHistory.map((answer: any) => {
      const value = Buffer.from(answer.value);
      const price = value.readBigInt64LE(0);
      const conf = value.readBigUInt64LE(8);
      const expo = value.readInt32LE(16);

      const decimalPrice = Number(price) * Math.pow(10, expo);
      const decimalConf = Number(conf) * Math.pow(10, expo);

      return {
        value: decimalPrice,
        timestamp: answer.timestamp.toNumber(),
        confidence: decimalConf,
        status: PriceStatus.Trading, // Historical data assumed trading
      };
    });
  }
}

/**
 * Helper functions for encoding data into Answer.value format
 */
export class OracleEncoder {
  /**
   * Encode price data for Pyth compatibility
   * Layout: [price: i64][conf: u64][expo: i32][reserved: 12 bytes]
   */
  static encodePythPrice(price: bigint, conf: bigint, expo: number): Buffer {
    const buffer = Buffer.alloc(32);
    buffer.writeBigInt64LE(price, 0);
    buffer.writeBigUInt64LE(conf, 8);
    buffer.writeInt32LE(expo, 16);
    // Remaining 12 bytes are zero (reserved)
    return buffer;
  }

  /**
   * Encode aggregator data for Switchboard compatibility
   * Layout: [value: i64][std_dev: i64][min: i64][max: i64]
   */
  static encodeSwitchboardResult(
    value: bigint,
    stdDev: bigint,
    min: bigint,
    max: bigint
  ): Buffer {
    const buffer = Buffer.alloc(32);
    buffer.writeBigInt64LE(value, 0);
    buffer.writeBigInt64LE(stdDev, 8);
    buffer.writeBigInt64LE(min, 16);
    buffer.writeBigInt64LE(max, 24);
    return buffer;
  }

  /**
   * Encode generic data (any format)
   */
  static encodeGeneric(data: Buffer): Buffer {
    if (data.length > 32) {
      throw new Error("Data exceeds 32 bytes");
    }
    const buffer = Buffer.alloc(32);
    data.copy(buffer, 0);
    return buffer;
  }
}

/**
 * Example usage:
 *
 * ```typescript
 * const provider = AnchorProvider.env();
 * const program = anchor.workspace.Molpha;
 * const client = new MolphaOracleClient(program, provider.connection);
 *
 * // Get Pyth-style price
 * const pythPrice = await client.getPythPrice(feedAddress);
 * console.log(`Price: ${pythPrice.price} * 10^${pythPrice.expo}`);
 *
 * // Get simple decimal price
 * const price = await client.getPrice(feedAddress);
 * console.log(`Price: $${price.value} (status: ${price.status})`);
 *
 * // Get price with staleness check (max 60 seconds old)
 * const freshPrice = await client.getPriceNoOlderThan(feedAddress, 60);
 * ```
 */
