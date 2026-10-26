// An item the fighter generates: a projectile of its own, riding the source's item box until it
// is thrown. The item's own script takes the hold and the throw up as asks on its exports
// (`__holder`, `__askState`, `__askActive`), in its own scope.

var Items = {};

// the content an item's name is converted to
Items.content = function(name) { return AS3.at(PeptideRuntime.data.pkg.contents, name); };

// where the item box is on the frame the fighter shows, in the source's units from its origin
Items.box = function(me) {
	var st = PeptideRuntime.shownClip();
	var p = st == null ? null : PeptideRuntime.pieceAt(st.__sym, st.currentFrame);
	if (p == null || p.itemBox.length == 0) return { x: 0, y: 0 };
	var k = st.currentFrame - p.from;
	var at = p.itemBox[0];
	for (b in p.itemBox) if (b[0] <= k) at = b;
	return { x: at[1], y: at[2] };
};

// the engine's object placed at the item box
Items.place = function(me, it) {
	var fm = me.__fm;
	var b = Items.box(me);
	var s = PeptideRuntime.data.units.size;
	it.setX(fm.getX() + fm.flipX(b.x * s));
	it.setY(fm.getY() + b.y * s);
	fm.exports.__ibx = b.x;
	fm.exports.__iby = b.y;
};

Items.held = function(me) {
	if (me.__item != null && me.__item.__fm.isDisposed()) me.__item = null;
	return me.__item;
};

Items.generate = function(me, name) {
	var fm = me.__fm;
	var id = Items.content(AS3.str(name));
	var content = id == null ? null : fm.getResource().getContent(id);
	if (content == null) { PeptideRuntime.report("no item " + AS3.str(name)); return null; }
	var it = match.createProjectile(content, fm);
	if (it == null) return null;
	// in the hand from its first frame: over the floor its body would meet where it is made,
	// and touching nothing until thrown
	if (it.exports != null) it.exports.__ghostDeclared = it.getProjectileStat("ghost") == true;
	it.updateProjectileStats({ ghost: true });
	Items.place(me, it);
	if (it.exports != null) { it.exports.__holder = fm; it.exports.__isItem = true; it.exports.__np = true; Items.ask(it.exports, "hold"); }
	// the item's own object when its script already runs (its class's methods are the source's to
	// call: a bob-omb's explode), else a plain one
	var item = it.exports == null ? null : it.exports.__source;
	if (item == null) { item = {}; PeptideItem.build(item); }
	item.__fm = it;
	item.__holder = me;
	item.__name = AS3.str(name);
	me.__item = item;
	return item;
};

// thrown the way the holder holds the stick: up, down, or along its facing
Items.direction = function(me) {
	var fm = me.__fm;
	var c = fm.getHeldControls();
	if (c != null && c.UP) return 90;
	if (c != null && c.DOWN) return 270;
	if (c != null && ((c.LEFT && fm.isFacingRight()) || (c.RIGHT && !fm.isFacingRight()))) return 158;
	return 12;
};

Items.toss = function(me, angle) {
	var item = Items.held(me);
	if (item == null) return;
	var it = item.__fm;
	var fm = me.__fm;
	if (angle == null) angle = Items.direction(me);
	var a = AS3.num(angle) * Math.PI / 180;
	var speed = (it.exports != null && it.exports.__tossSpeed != null) ? it.exports.__tossSpeed : PeptideRuntime.data.units.itemToss;
	var sp = speed * PeptideRuntime.data.units.speed;
	Items.place(me, it);
	if (it.exports != null) { it.exports.__holder = null; Items.ask(it.exports, "toss"); }
	it.setXSpeed(fm.flipX(Math.cos(a) * sp));
	it.setYSpeed(-Math.sin(a) * sp);
	me.__item = null;
};

// a throw asked for: the fighter's own throw animation for the way it holds the stick (its
// frames call tossItem), when it has one; else the item is thrown from here
Items.throwAsked = function(me) {
	if (Items.held(me) == null) return;
	var fm = me.__fm;
	var d = Items.direction(me);
	var w = (d == 90) ? "up" : (d == 270) ? "down" : (d == 158) ? "back" : "forward";
	var air = fm.isOnFloor() ? "" : "_air";
	for (n in ["item_throw" + air + "_" + w, "item_throw" + air]) {
		if (PeptideRuntime.pieceOf(n) != null) { if (fm.getAnimation() != n) fm.playAnimation(n); return; }
	}
	Items.toss(me, null);
};

// let go where it is, without a throw (zDropped: let go by a grab or a hit)
Items.drop = function(me, ?z) {
	var item = Items.held(me);
	if (item == null) return;
	var p = item.__fm;
	Items.place(me, p);
	p.exports.__holder = null;
	Items.ask(p.exports, "toss");
	p.exports.__zDropped = AS3.truthy(z);
	p.setXSpeed(0);
	p.setYSpeed(0);
	me.__item = null;
};

Items.remove = function(me) {
	var item = Items.held(me);
	if (item != null) item.__fm.kill();
	me.__item = null;
};

// asked of an item, in its own script: the ask it publishes (`__ask`), else a field its own frame
// reads (one made without the runtime)
Items.ask = function(ex, kind, ?v) {
	if (ex == null) return;
	if (ex.__ask != null) { ex.__ask(kind, v); return; }
	if (kind == "attack") ex.__askAttack = v; else if (kind == "interrupt") ex.__askInterrupt = v; else ex.__askState = kind;
};

// every frame the fighter holds one, the item rides the box
Items.frame = function(me) {
	var item = Items.held(me);
	if (item == null) { me.__itemLabel = null; return; }
	Items.place(me, item.__fm);
	// an attack begun while holding it is put to the item first (its frame interrupt)
	var label = me.__mc == null ? null : me.__mc.currentLabel;
	if (label != me.__itemLabel) {
		me.__itemLabel = label;
		if (label != null && AS3.at(PeptideRuntime.data.pkg.attacks, label) != null) {
			Items.ask(item.__fm.exports, "interrupt", { targetFrame: label, character: me.__fm, cStick: false, zair: false });
		}
	}
};

// class PeptideItem extends PeptideGameObject
var PeptideItem = { name: "PeptideItem" };
PeptideItem.build = function(me) {
	PeptideGameObject.build(me);
	me.__is.push("PeptideItem");
	me.__type = "SSF2Item";
};
// what the item declares: its name is its linkage, the rest its own stats
PeptideItem.getItemStat = function(me, k) {
	if (k == "linkage_id") return me.__name;
	return AS3.at(AS3.at(PeptideRuntime.data.pkg.items, me.__name), k);
};
// the item's own attacks (its toss, its idle), when its stats let it: started in its own script,
// directly when this is that script's item, else asked of it. the holder
// keeps it and plays nothing
PeptideItem.attack = function(me, n) {
	if (me == PeptideRuntime.object) { PeptideGameObject.startAttack(me, n); return; }
	Items.ask(me.__fm.exports, "attack", n);
};
PeptideItem.toHeld = function(me) { var h = me.__holder; if (h == null) return; me.__fm.exports.__holder = h.__fm; Items.ask(me.__fm.exports, "hold"); h.__item = me; };
// what its own code changes of its stats; the ones this engine keeps are handed on
PeptideItem.updateItemStats = function(me, o) {
	var u = PeptideRuntime.data.units;
	if (me.__stats == null) me.__stats = new haxe.ds.StringMap();
	for (k in AS3.keysOf(o)) {
		var v = AS3.at(o, k);
		me.__stats.set(k, v);
		if (k == "gravity") me.__fm.updateProjectileStats({ gravity: AS3.num(v) * u.speed * 0.5 });
		else if (k == "max_gravity" || k == "maxgravity") me.__fm.updateProjectileStats({ terminalVelocity: AS3.num(v) * u.speed });
		else if (k == "ghost") me.__fm.updateProjectileStats({ ghost: AS3.truthy(v) });
	}
};
PeptideItem.init = function(me, ?api) {};
// gone from the match, as any object its code ends
PeptideItem.destroy = function(me) { Objects.destroy(me); };
