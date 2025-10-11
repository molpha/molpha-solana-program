// Jest setup file
import { expect } from "@jest/globals";

// Make Jest's expect available globally
global.expect = expect;

// Configure Jest timeout for Solana tests
jest.setTimeout(60000);

// Add Mocha-style hooks for compatibility
global.before = beforeAll;
global.after = afterAll;
global.beforeEach = beforeEach;
global.afterEach = afterEach;
