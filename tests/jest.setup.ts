// Jest setup file
import { expect } from "@jest/globals";

// Make Jest's expect available globally
global.expect = expect;

// Bankrun + startAnchor can exceed 60s on cold SBF builds.
jest.setTimeout(180000);

// Add Mocha-style hooks for compatibility
global.before = beforeAll;
global.after = afterAll;
global.beforeEach = beforeEach;
global.afterEach = afterEach;
