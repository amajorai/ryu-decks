// Typed calls to the `ryu-decks` sidecar through the host's `app:http` bridge.
// Paths are relative to the app's `/api/decks` mount. Every response is parsed,
// so a sidecar that answers with something unexpected surfaces as an error
// instead of rendering half a deck.

import { z } from "zod";

const familySchema = z.enum([
	"elgato",
	"ulanzi",
	"work_louder",
	"loupedeck",
	"mirabox",
	"apple",
	"infinitton",
	"xkeys",
	"contour",
	"logitech",
	"mountain",
	"xencelabs",
	"blackmagic",
	"203systems",
	"framework",
	"vec",
	"tourbox",
	"corsair",
	"cooler_master",
	"3dconnexion",
	"qmk",
	"yamaha",
	"microsoft",
	"prehkeytec",
	"dji",
	"haute42",
	"native_instruments",
	"monome",
	"lamp_array",
	"wooting",
	"sony",
	"adafruit",
	"gamesir",
	"steelseries",
	"griffin",
	"novation",
	"akai",
	"ableton",
	"behringer",
	"korg",
	"arturia",
	"djtechtools",
	"intech",
	"presonus",
	"faderfox",
	"livid",
	"allen_heath",
	"intuitive_instruments",
	"maudio",
	"icon",
	"zoom",
	"mackie",
]);

export const modelSchema = z.object({
	// The Touch Bar has no grid: zero rows, columns and grid keys.
	cols: z.number().int().nonnegative(),
	dials: z.number().int().nonnegative(),
	face_size: z.number().int().positive().nullable(),
	/** Width and height of a grid key's face (tall on the Stream Deck Studio). */
	key_face: z
		.tuple([z.number().int().positive(), z.number().int().positive()])
		.nullable(),
	family: familySchema,
	id: z.string(),
	key_lights: z.boolean(),
	key_screens: z.boolean(),
	keys: z.number().int().nonnegative(),
	name: z.string(),
	product_ids: z.array(z.string()),
	rows: z.number().int().nonnegative(),
	extra_keys: z.number().int().nonnegative(),
	/** Touch-strip keys, numbered after the grid keys. */
	strip_keys: z.number().int().nonnegative(),
	/** Width and height of a strip key's face. */
	strip_face: z
		.tuple([z.number().int().positive(), z.number().int().positive()])
		.nullable(),
	/** Under the grid, or as columns either side of it (Loupedeck side strips). */
	strip_placement: z.enum(["below", "sides", "above"]),
	/** Side of the round wheel screen's face (the Loupedeck CT). */
	wheel_face: z.number().int().positive().nullable(),
	/** Dial lights Ryu can colour: one per dial, one shared, or none. */
	dial_lights: z.enum(["none", "per_dial", "shared"]),
	/** Lights on the deck itself (the M18's 24); they take one colour. */
	deck_lights: z.number().int().nonnegative(),
	/** Whether the deck takes a brightness setting (the DisplayPad does not). */
	brightness: z.boolean(),
	/**
	 * Legacy X-keys lights show only blue, red or both; the Resolve editors'
	 * lights are on or off; the ATEM Micro Panel's switch red, green and blue
	 * each on or off; the Yamaha CC121MK2's take the nearest of ten colours;
	 * a monome grid's glow in one colour at sixteen levels; a bi-colour
	 * Launchpad's and a Launch Control's mix four levels each of red and
	 * green (a Launch Control's arrows and side buttons are only on or off);
	 * a Launchkey's are "rgb" but show the nearest of 128 palette colours, and
	 * some of its buttons only off, half or full; an APC mini MK2's pads are
	 * "rgb", while its scene (green) and track (red) buttons are only on or off;
	 * the first APC mini's and first APC Key 25's pads show off, green, red or
	 * yellow (amber), and their scene and track buttons are only on or off;
	 * a Push 2's are "rgb" but show the nearest of its 127 palette colours,
	 * and 34 of its buttons are white, showing only brightness levels; a
	 * Novation Dicer's show off, or dim or bright green or orange.
	 */
	key_light_colors: z.enum([
		"rgb",
		"blue_red",
		"on_off",
		"basic_rgb",
		"palette",
		"levels",
		"red_green",
		"green_red_yellow",
		"green_orange",
	]),
	/** The keys that have a light, when only some do. */
	lit_keys: z.array(z.number().int().nonnegative()).nullable(),
	/** The legend printed on each key (the Blackmagic panels). */
	key_names: z.array(z.string()).nullable(),
	/** Each key's cell in the grid, when keys do not fill it in reading order. */
	key_cells: z.array(z.number().int().nonnegative()).nullable(),
	/** Width and height of the Stream Deck Neo's info screen. */
	info_face: z
		.tuple([z.number().int().positive(), z.number().int().positive()])
		.nullable(),
	/** Width and height of an extra key's own small screen (the Mirabox N1). */
	extra_face: z
		.tuple([z.number().int().positive(), z.number().int().positive()])
		.nullable(),
	vendor_id: z.string(),
});

export const deviceSchema = z.object({
	bound_keys: z.number().int().nonnegative(),
	/** One entry per dial light colour (`null` = off). */
	dial_colors: z.array(z.string().nullable()),
	/** The colour of the deck's own lights (`null` = off or none). */
	deck_light: z.string().nullable(),
	brightness: z.number().int().min(0).max(100).nullable(),
	connected: z.boolean(),
	error: z.string().nullable(),
	id: z.string(),
	/**
	 * Whether the keys are unlocked, on a connected deck that locks them (a
	 * Vial keyboard, until its unlock keys are held).
	 */
	lock: z
		.object({
			keys: z.array(z.number().int().nonnegative()),
			/** Some unlock keys are not on the drawn layout: unlock it in Vial. */
			keys_off_layout: z.boolean(),
			progress: z.number().int().min(0).max(100),
			unlocked: z.boolean(),
			unlocking: z.boolean(),
		})
		.nullable(),
	model: modelSchema,
	serial: z.string().nullable(),
});

const devicesResponseSchema = z.object({
	devices: z.array(deviceSchema),
	events: z.object({
		dial_turned: z.string(),
		key_pressed: z.string(),
		strip_swiped: z.string(),
	}),
	hid: z.object({ error: z.string().nullable(), running: z.boolean() }),
});

export const actionSchema = z.discriminatedUnion("type", [
	z.object({ type: z.literal("none") }),
	z.object({
		agent_id: z.string().nullable().optional(),
		prompt: z.string(),
		type: z.literal("ask_agent"),
	}),
]);

export const keySchema = z.object({
	action: actionSchema,
	color: z.string(),
	face: z.string().nullable(),
	key: z.number().int().nonnegative(),
	label: z.string(),
	updated_at: z.number(),
});

export const activitySchema = z.object({
	action: z.string(),
	at: z.string(),
	control: z.string(),
	detail: z.string().nullable(),
	device_id: z.string(),
	key: z.number().int(),
	label: z.string().nullable(),
	model_name: z.string(),
	outcome: z.enum([
		"event_raised",
		"sent",
		"pending_approval",
		"unconfirmed",
		"ignored",
		"failed",
	]),
	source: z.enum(["device", "test"]),
});

export type DeckModel = z.infer<typeof modelSchema>;
export type Deck = z.infer<typeof deviceSchema>;
export type DecksOverview = z.infer<typeof devicesResponseSchema>;
export type KeyAction = z.infer<typeof actionSchema>;
export type KeyBinding = z.infer<typeof keySchema>;
export type Activity = z.infer<typeof activitySchema>;

export interface KeyInput {
	action: KeyAction;
	color: string;
	face: string | null;
	label: string;
}

const INVALID_RESPONSE = "Decks returned a response it could not read";

function bridge() {
	const app = globalThis.window?.ryu?.app;
	if (!app) {
		throw new Error("Decks is not connected to Ryu. Open it from the Ryu app.");
	}
	return app;
}

async function request<T>(
	schema: z.ZodType<T>,
	input: {
		body?: unknown;
		method?: "DELETE" | "GET" | "POST" | "PUT";
		path: string;
	}
): Promise<T> {
	const parsed = schema.safeParse(await bridge().request(input));
	if (!parsed.success) {
		throw new Error(INVALID_RESPONSE);
	}
	return parsed.data;
}

function deckPath(deckId: string): string {
	return `/devices/${encodeURIComponent(deckId)}`;
}

export function listDecks(): Promise<DecksOverview> {
	return request(devicesResponseSchema, { path: "/devices" });
}

export function listModels(): Promise<DeckModel[]> {
	return request(z.object({ models: z.array(modelSchema) }), {
		path: "/models",
	}).then((response) => response.models);
}

export function listKeys(deckId: string): Promise<KeyBinding[]> {
	return request(z.object({ keys: z.array(keySchema) }), {
		path: `${deckPath(deckId)}/keys`,
	}).then((response) => response.keys);
}

export function saveKey(
	deckId: string,
	key: number,
	input: KeyInput
): Promise<KeyBinding> {
	return request(keySchema, {
		body: input,
		method: "PUT",
		path: `${deckPath(deckId)}/keys/${key}`,
	});
}

export function clearKey(deckId: string, key: number): Promise<boolean> {
	return request(z.object({ removed: z.boolean() }), {
		method: "DELETE",
		path: `${deckPath(deckId)}/keys/${key}`,
	}).then((response) => response.removed);
}

export function pressKey(deckId: string, key: number): Promise<Activity> {
	return request(activitySchema, {
		method: "POST",
		path: `${deckPath(deckId)}/keys/${key}/press`,
	});
}

export function setBrightness(
	deckId: string,
	percent: number
): Promise<number> {
	return request(z.object({ brightness: z.number() }), {
		body: { percent },
		method: "PUT",
		path: `${deckPath(deckId)}/brightness`,
	}).then((response) => response.brightness);
}

export function setDialLight(
	deckId: string,
	dial: number,
	color: string | null
): Promise<string | null> {
	return request(z.object({ color: z.string().nullable() }), {
		body: { color },
		method: "PUT",
		path: `${deckPath(deckId)}/dials/${dial}/light`,
	}).then((response) => response.color);
}

export function setDeckLight(
	deckId: string,
	color: string | null
): Promise<string | null> {
	return request(z.object({ color: z.string().nullable() }), {
		body: { color },
		method: "PUT",
		path: `${deckPath(deckId)}/light`,
	}).then((response) => response.color);
}

/** Starts a Vial keyboard's unlock; the user then holds its unlock keys. */
export function unlockDeck(deckId: string): Promise<boolean> {
	return request(z.object({ started: z.boolean() }), {
		method: "POST",
		path: `${deckPath(deckId)}/unlock`,
	}).then((response) => response.started);
}

export function listActivity(): Promise<Activity[]> {
	return request(z.object({ activity: z.array(activitySchema) }), {
		path: "/activity",
	}).then((response) => response.activity);
}
