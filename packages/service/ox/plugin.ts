import { definePlugin } from "@oxlint/plugins";
import paddingBetweenStatementsRule from "./lint-rules/padding-between-statements.ts";

export default definePlugin({
	meta: { name: "spacing" },
	rules: {
		"padding-between-statements": paddingBetweenStatementsRule,
	},
});
