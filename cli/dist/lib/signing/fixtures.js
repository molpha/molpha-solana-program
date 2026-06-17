"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.loadVerifyFixture = loadVerifyFixture;
const utils_1 = require("../utils");
function loadVerifyFixture(filePath) {
  return (0, utils_1.readJsonFile)(filePath);
}
