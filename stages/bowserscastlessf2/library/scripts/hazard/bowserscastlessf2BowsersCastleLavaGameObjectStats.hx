// GameObjectStats for bowserscastlessf2BowsersCastleLava
{
	spriteContent: self.getResource().getContent("bowserscastlessf2BowsersCastleLava"),
	initialState: PState.ACTIVE,
	stateTransitionMapOverrides: [
		PState.ACTIVE => { animation: "gameObjectIdle" }
	],
	baseScaleX: 1,
	baseScaleY: 1,
	weight: 100,
	gravity: 0,
	friction: 0,
	immovable: true,  // a stage hazard shoves fighters, never the reverse (windboxes included)
}