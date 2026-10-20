// The site's only script: the vanilla controllers, loaded as one module after the static HTML.
// There is no React on the client; every page is plain HTML that works before (or without) this.
import { start } from "./main";

start();
