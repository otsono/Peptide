// The pieces a stage's boot puts down and the upkeep its frame runs, as Fraymakers code: a stage
// written natively carries only the blocks it uses (`emit/stage.rs`). The conversion writes the
// boot as one call per piece, in the order the stage puts things down.

// ---- sized
//: the handles the boot files by index (a piece riding another is found by it)
var bootBg = [];
function bootSized(objects, elements) {
	while (bootBg.length < elements) bootBg.push(null);
}

// ---- structure
//: a structure the stage entity carries
function bootStructure(id, name) {
	var h = match.createStructure(self.getResource().getContent(id));
	__made.push({ id: id, k: "s", o: h });
}

// ---- container
//: where an element's view goes
function bootContainer(at) {
	if (at.pan == true) return panContainer(at.px, at.py);
	if (at.plane == "camera") return match.getCamera().getBackgroundContainers()[at.n];
	if (at.plane == "background") return self.getBackgroundContainer();
	if (at.plane == "foreground") return self.getForegroundStructuresContainer();
	return self.getBackgroundBehindContainer();
}

// ---- pan
//: one container per pan rate, moved against the camera by that rate: a piece at 0 stays with the
//: view, one at 1 stays in the world
var panRates = [];
var panConts = [];
var panX0 = null;
var panY0 = null;
function panContainer(px, py) {
	for (i in 0...panRates.length) if (panRates[i].x == px && panRates[i].y == py) return panConts[i];
	var c = Container.create();
	self.getBackgroundBehindContainer().addChild(c);
	panRates.push({ x: px, y: py });
	panConts.push(c);
	return c;
}
// the pan containers, against where the camera started (PAN_X, PAN_Y: the size they centre on)
function panUpkeep() {
	if (panConts.length == 0) return;
	var cam = match.getCamera();
	if (cam == null) return;
	if (panX0 == null) { panX0 = cam.getX(); panY0 = cam.getY(); }
	var dx = cam.getX() - panX0;
	var dy = cam.getY() - panY0;
	for (i in 0...panConts.length) {
		panConts[i].x = cam.getX() - PAN_X * 0.5 - dx * panRates[i].x;
		panConts[i].y = cam.getY() - PAN_Y * 0.5 - dy * panRates[i].y;
	}
}

// ---- effect
//: a looping piece: an effect in a container of its own, which the stage's timeline shows and hides
function bootEffect(e) {
	var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(e.id), animation: "active", x: e.x, y: e.y, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
	var c = Container.create();
	bootContainer(e.at).addChild(c);
	if (v != null) {
		c.addChild(v.getViewRootContainer());
		m_owned.push(v);
		__made.push({ id: e.id, k: "v", o: v });
	}
	if (e.shown != null) m_shown.push({ c: c, i: 0, at: 0, on: e.shown });
	if (e.tint != null && v != null) {
		var sh = new RgbaColorShader(1, 1, 1, 1, 0, 0, 0, 0);
		v.addShader(sh);
		m_tint.push({ s: sh, i: 0, j: -1, at: 0, len: e.tlen, k: e.tint });
	}
	if (e.alpha != null && v != null) v.getViewRootContainer().alpha = e.alpha;
	if (e.hidden == true && v != null) v.setVisible(false);
	bootBg[e.i] = v;
}

// ---- cam_alpha
//: a camera background's opacity
function bootCamAlpha(i, a) {
	var cbg = match.getCamera().getBackgroundContainers();
	if (cbg != null && cbg.length > i) cbg[i].alpha = a;
}

// ---- cam_fade
//: a camera background's opacity as a schedule the frame runs
function bootCamFade(i, len, k) {
	var cbg = match.getCamera().getBackgroundContainers();
	if (cbg != null && cbg.length > i) m_alpha.push({ c: cbg[i], i: 0, j: -1, at: 0, len: len, k: k });
}

// ---- shown
//: shown over runs of [from, to) on the clock: a cursor per container, moved on only forward
function shownUpkeep(t) {
	for (e in m_shown) {
		if (t < e.at) e.i = 0;
		e.at = t;
		while (e.i < e.on.length && t >= e.on[e.i][1]) e.i++;
		e.c.visible = e.i < e.on.length && t >= e.on[e.i][0];
	}
}

// ---- key
//: opacity and colour hold each key until the next, on the piece's own loop
function keyAt(e, c) {
	if (c < e.at) e.i = 0;
	e.at = c;
	while (e.i + 1 < e.k.length && c >= e.k[e.i + 1][0]) e.i++;
}

// ---- alphas
function alphasUpkeep(t) {
	for (e in m_alpha) {
		var c = e.len > 0 ? t % e.len : t;
		keyAt(e, c);
		e.c.alpha = e.k[e.i][1];
	}
}

// ---- tints
function tintsUpkeep(t) {
	for (e in m_tint) {
		var c = e.len > 0 ? t % e.len : t;
		keyAt(e, c);
		var k = e.k[e.i];
		e.s.redMultiplier = k[1];
		e.s.greenMultiplier = k[2];
		e.s.blueMultiplier = k[3];
		e.s.redOffset = k[4];
		e.s.greenOffset = k[5];
		e.s.blueOffset = k[6];
	}
}
