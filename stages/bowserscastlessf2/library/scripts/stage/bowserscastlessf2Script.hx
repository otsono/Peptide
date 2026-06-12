// API Script for bowserscastlessf2 (converted from SSF2)

var m_hazardsSpawned = false;
function initialize() {
	// animated stage clips play + loop on the timeline
}
function update() {
	if (!m_hazardsSpawned) {
		var chars = match.getCharacters();
		if (chars.length > 0) {
			m_hazardsSpawned = true;
			var owner = null;
			match.createStructure(self.getResource().getContent("bowserscastlessf2platform0"));
			match.createStructure(self.getResource().getContent("bowserscastlessf2platform1"));
			match.createStructure(self.getResource().getContent("bowserscastlessf2thwompdeck"));
			match.createStructure(self.getResource().getContent("bowserscastlessf2thwompceiling"));
			var _hz0 = match.createCustomGameObject(self.getResource().getContent("bowserscastlessf2BowsersCastleLava"), owner);
			if (_hz0 != null) {
				_hz0.setX(772.9);
				_hz0.setY(1057.8);
			}
			var _hz1 = match.createCustomGameObject(self.getResource().getContent("bowserscastlessf2Thwomp"), owner);
			if (_hz1 != null) {
				_hz1.setX(686.7);
				_hz1.setY(1648.3);
			}
			var _bg0 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowserspectator"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg0 != null) {
				self.getBackgroundBehindContainer().addChild(_bg0.getSprite());
			}
			var _bg1 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg1"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg1 != null) {
				self.getBackgroundBehindContainer().addChild(_bg1.getSprite());
			}
			var _bg2 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg2"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg2 != null) {
				self.getBackgroundBehindContainer().addChild(_bg2.getSprite());
			}
			var _bg3 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg3"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg3 != null) {
				self.getBackgroundBehindContainer().addChild(_bg3.getSprite());
			}
			var _bg4 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg4"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg4 != null) {
				self.getBackgroundBehindContainer().addChild(_bg4.getSprite());
			}
			var _bg5 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg5"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg5 != null) {
				self.getBackgroundBehindContainer().addChild(_bg5.getSprite());
			}
			var _bg6 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg6"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg6 != null) {
				self.getBackgroundBehindContainer().addChild(_bg6.getSprite());
			}
			var _bg7 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg7"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg7 != null) {
				self.getBackgroundBehindContainer().addChild(_bg7.getSprite());
			}
			var _bg8 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg8"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg8 != null) {
				self.getBackgroundBehindContainer().addChild(_bg8.getSprite());
			}
			var _bg9 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg9"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg9 != null) {
				self.getBackgroundBehindContainer().addChild(_bg9.getSprite());
			}
			var _bg10 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg10"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg10 != null) {
				self.getBackgroundBehindContainer().addChild(_bg10.getSprite());
			}
			var _bg11 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg11"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg11 != null) {
				self.getBackgroundBehindContainer().addChild(_bg11.getSprite());
			}
			var _bg12 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg12"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg12 != null) {
				self.getBackgroundBehindContainer().addChild(_bg12.getSprite());
			}
			var _bg13 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg13"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg13 != null) {
				self.getBackgroundBehindContainer().addChild(_bg13.getSprite());
			}
			var _bg14 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg14"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg14 != null) {
				self.getBackgroundBehindContainer().addChild(_bg14.getSprite());
			}
			var _bg15 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg15"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg15 != null) {
				self.getBackgroundBehindContainer().addChild(_bg15.getSprite());
			}
			var _bg16 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_torches_lit_bg16"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg16 != null) {
				self.getBackgroundBehindContainer().addChild(_bg16.getSprite());
			}
			var _bg17 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg1"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg17 != null) {
				self.getBackgroundBehindContainer().addChild(_bg17.getSprite());
			}
			var _bg18 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg2"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg18 != null) {
				self.getBackgroundBehindContainer().addChild(_bg18.getSprite());
			}
			var _bg19 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg3"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg19 != null) {
				self.getBackgroundBehindContainer().addChild(_bg19.getSprite());
			}
			var _bg20 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg4"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg20 != null) {
				self.getBackgroundBehindContainer().addChild(_bg20.getSprite());
			}
			var _bg21 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg5"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg21 != null) {
				self.getBackgroundBehindContainer().addChild(_bg21.getSprite());
			}
			var _bg22 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg6"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg22 != null) {
				self.getBackgroundBehindContainer().addChild(_bg22.getSprite());
			}
			var _bg23 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg7"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg23 != null) {
				self.getBackgroundBehindContainer().addChild(_bg23.getSprite());
			}
			var _bg24 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg8"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg24 != null) {
				self.getBackgroundBehindContainer().addChild(_bg24.getSprite());
			}
			var _bg25 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg9"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg25 != null) {
				self.getBackgroundBehindContainer().addChild(_bg25.getSprite());
			}
			var _bg26 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg10"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg26 != null) {
				self.getBackgroundBehindContainer().addChild(_bg26.getSprite());
			}
			var _bg27 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg11"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg27 != null) {
				self.getBackgroundBehindContainer().addChild(_bg27.getSprite());
			}
			var _bg28 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg12"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg28 != null) {
				self.getBackgroundBehindContainer().addChild(_bg28.getSprite());
			}
			var _bg29 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg13"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg29 != null) {
				self.getBackgroundBehindContainer().addChild(_bg29.getSprite());
			}
			var _bg30 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg14"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg30 != null) {
				self.getBackgroundBehindContainer().addChild(_bg30.getSprite());
			}
			var _bg31 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg15"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg31 != null) {
				self.getBackgroundBehindContainer().addChild(_bg31.getSprite());
			}
			var _bg32 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_clocktown_torchembersleft_bg16"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg32 != null) {
				self.getBackgroundBehindContainer().addChild(_bg32.getSprite());
			}
			var _bg33 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_podoboos_bg"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg33 != null) {
				self.getBackgroundBehindContainer().addChild(_bg33.getSprite());
			}
			var _bg34 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("bowserscastlessf2_bowsers_bubbles_bg"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg34 != null) {
				self.getBackgroundBehindContainer().addChild(_bg34.getSprite());
			}
		}
	}
}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}