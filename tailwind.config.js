/** @type {import('tailwindcss').Config} */
module.exports = {
	content: ["src/**/*.{html,rs}"],
	safelist: ["dark"],
	daisyui: {
		themes: ["light", "dark"],
	},
	plugins: [require("daisyui")],
};
