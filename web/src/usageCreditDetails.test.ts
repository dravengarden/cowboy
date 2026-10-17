import { expect, test } from "bun:test";
import { creditMoney, usageCreditDetails } from "./usageCreditDetails";

test("credit amounts use the reported currency's minor unit", () => {
  expect(creditMoney(1234, "USD")).toBe(
    new Intl.NumberFormat(undefined, { style: "currency", currency: "USD" })
      .format(12.34),
  );
  expect(creditMoney(1234, "JPY")).toBe(
    new Intl.NumberFormat(undefined, { style: "currency", currency: "JPY" })
      .format(1234),
  );
  expect(creditMoney(0, undefined)).toBeUndefined();
  expect(creditMoney(-1, "USD")).toBeUndefined();
});

test("credit status retains off and zero without inventing a balance", () => {
  const rows = usageCreditDetails({
    usageCredits: {
      enabled: false,
      balanceMinor: 0,
      balanceCurrency: "USD",
      autoReloadEnabled: false,
      spentMinor: 500,
    },
  });
  expect(rows).toEqual([{ label: "Usage credits", value: "Off" }, {
    label: "Balance",
    value: creditMoney(0, "USD"),
  }, { label: "Auto reload", value: "Off" }]);
  expect(usageCreditDetails({ usageCredits: { balanceStatus: "unavailable" } }))
    .toEqual([{ label: "Balance", value: "Unavailable" }]);
  expect(usageCreditDetails(undefined)).toEqual([]);
});

test("held resets remain visible without claiming they are usable", () => {
  expect(
    usageCreditDetails({
      rateLimitResetCredits: {
        grants: [{ title: "Weekly", remaining: 2, status: "held" }],
      },
    }),
  ).toEqual([{ label: "Weekly resets", value: "2 not currently usable" }]);
});
