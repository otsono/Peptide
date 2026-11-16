// The source's engine tells a projectile it has met the one who threw it (a boomerang caught on
// its way back), once as they meet and again only after they part (compile/native.rs takes this
// block where a projectile's code listens for it).

// ---- owner
var touchingOwner = self.makeObject(false);

// each frame: its listeners told as it meets its owner
function touchFrame() {
	var owner = self.getOwner();
	if (owner == null || owner.isDisposed()) return;
	var hits = self.collisionTest(owner, CollisionBoxType.HIT, CollisionBoxType.HURT, true);
	if (hits == null || hits.length == 0) hits = self.collisionTest(owner, CollisionBoxType.HURT, CollisionBoxType.HURT, true);
	var touching = hits != null && hits.length > 0;
	if (touching && !touchingOwner.get()) {
		var e = { data: { foe: owner } };
		// {cases}
	}
	touchingOwner.set(touching);
}
