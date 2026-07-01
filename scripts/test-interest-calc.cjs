const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { rmSync } = require("node:fs");
const { join } = require("node:path");
const { tmpdir } = require("node:os");

const outDir = join(tmpdir(), "caseboard-interest-calc-test");
rmSync(outDir, { recursive: true, force: true });

execFileSync(
  "pnpm",
  [
    "exec",
    "tsc",
    "--target",
    "ES2020",
    "--module",
    "commonjs",
    "--moduleResolution",
    "node",
    "--outDir",
    outDir,
    "--rootDir",
    "src/modules/tools/lib",
    "--skipLibCheck",
    "src/modules/tools/lib/interestCalc.ts",
    "src/modules/tools/lib/lprData.ts",
  ],
  { stdio: "inherit" },
);

const {
  calcFiveStage,
  calculateInterestByPeriod,
  calculateInterestSegments,
  formatMoney,
} = require(join(outDir, "interestCalc.js"));
const { getLprForDate } = require(join(outDir, "lprData.js"));

assert.equal(getLprForDate("2026-06-24", "1y"), 3);
assert.equal(getLprForDate("2026-06-24", "5y+"), 3.5);
assert.equal(formatMoney(14432.87), "14,432.87 元");
assert.equal(formatMoney(611712), "611,712.00 元");

const baseInterest = calculateInterestByPeriod(
  100000,
  "2024-08-11",
  "2024-09-02",
  "lpr",
  0,
  "1y",
);
assert.equal(baseInterest, 201.92);

const multipliedInterest = calculateInterestByPeriod(
  100000,
  "2024-08-11",
  "2024-09-02",
  "lpr",
  0,
  "1y",
  1.5,
);
assert.equal(multipliedInterest, 302.88);

const [segment] = calculateInterestSegments(
  100000,
  "2024-08-11",
  "2024-09-02",
  "lpr",
  0,
  "1y",
  1.5,
);
assert.equal(segment.baseRate, 3.35);
assert.equal(segment.multiplier, 1.5);
assert.equal(segment.rate, 5.025);
assert.equal(segment.interest, 302.88);

const cappedInterest = calculateInterestByPeriod(
  100000,
  "2020-08-10",
  "2020-08-30",
  "hybrid",
  24,
  "1y",
  4,
);
assert.equal(cappedInterest, 1079.45);

const cappedSegments = calculateInterestSegments(
  100000,
  "2020-08-10",
  "2020-08-30",
  "hybrid",
  24,
  "1y",
  4,
);
assert.deepEqual(
  cappedSegments.map((s) => ({
    startDate: s.startDate,
    endDate: s.endDate,
    rateType: s.rateType,
    baseRate: s.baseRate,
    multiplier: s.multiplier,
    rate: s.rate,
    interest: s.interest,
  })),
  [
    {
      startDate: "2020-08-10",
      endDate: "2020-08-20",
      rateType: "hybrid",
      baseRate: 24,
      multiplier: 1,
      rate: 24,
      interest: 657.53,
    },
    {
      startDate: "2020-08-20",
      endDate: "2020-08-30",
      rateType: "hybrid",
      baseRate: 3.85,
      multiplier: 4,
      rate: 15.4,
      interest: 421.92,
    },
  ],
);

const execution = calcFiveStage(
  {
    id: 1,
    name: "测试案件",
    principal: 100000,
    rate: 0,
    rateType: "lpr",
    lprTerm: "1y",
    lprMultiplier: 1.5,
    startDate: "2024-08-11",
    endDate: "2024-09-02",
    litigationFee: 0,
    lawyerFee: 0,
    otherFee: 0,
  },
  [],
  false,
);
assert.equal(Math.round(execution.accumulatedInterest * 100) / 100, 302.88);
assert.equal(Math.round(execution.total * 100) / 100, 100302.88);
assert.equal(execution.finalInterestSegments[0].multiplier, 1.5);

console.log("interest calculation tests passed");
