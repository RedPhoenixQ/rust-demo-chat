/** @type {import('tailwindcss').Config} */
module.exports = {
	content: ["src/**/*.{html,rs}"],
	safelist: ["dark"],
	daisyui: {
		themes: ["light", "dark"],
	},
	plugins: [require("daisyui")],
	theme: {
		maxHeight: {
			screen: ["100vh", "100dvh"],
			vh: "100vh",
		},
		minHeight: {
			screen: ["100vh", "100dvh"],
			vh: "100vh",
		},
		height: {
			screen: ["100vh", "100dvh"],
			vh: "100vh",
		},
	},
};
