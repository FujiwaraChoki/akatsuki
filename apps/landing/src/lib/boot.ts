// Runs in <head> before first paint: applies the stored theme, marks the platform for ⌘/Ctrl
// labels, and adds `has-js` (JS runs) and `js` (JS runs and motion is allowed). Without the `js`
// class every entrance renders in its final state.
export const THEME_KEY = "ak-theme";
export const BOOT_SCRIPT = `(function(){var d=document.documentElement;try{var t=localStorage.getItem("${THEME_KEY}");if(t==="light"||t==="dark")d.setAttribute("data-theme",t)}catch(e){}try{var n=navigator,p=(n.userAgentData&&n.userAgentData.platform)||n.platform||"";if(/mac|iphone|ipad|ipod/i.test(p))d.setAttribute("data-platform","mac")}catch(e){}d.classList.add("has-js");if(!window.matchMedia||!matchMedia("(prefers-reduced-motion: reduce)").matches)d.classList.add("js","motion")})()`;
