// GameObjectStats for bowserscastlessf2hazard1
{
	spriteContent: self.getResource().getContent("bowserscastlessf2hazard1"),
	initialState: PState.ACTIVE,
	stateTransitionMapOverrides: [
		PState.ACTIVE => { animation: "gameObjectIdle" }
	],
	baseScaleX: 1,
	baseScaleY: 1,
	weight: 100,
	gravity: 0,
	friction: 0
}
