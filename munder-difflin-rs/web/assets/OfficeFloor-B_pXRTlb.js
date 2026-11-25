import { t as useTranslation, n as reactExports, l as jsxRuntimeExports } from "./index-ChdtDDPz.js";
import { g as colors, b as OFFICE_FLOOR, v as readThemeOverlay, z as savedThemeName, r as openFloorDataFeed, j as fitRect, m as floorPowerPreference, w as recordFloorRenderer, H as watchWindowOfficeAsleep, E as waitsOnHuman } from "./index--nBMM_aP.js";
import { Q as Sprite, X as Texture, a0 as Ticker, a3 as UPDATE_PRIORITY, k as Container, R as Rectangle, H as Graphics, W as Text, aG as toolIcon, U as StationFloor, an as floorPaused, a8 as applyFloorPause, a as Application, aB as planInitFailure, D as DEFAULT_MAX_INIT_RETRIES, r as FloorEdge, ay as installContextLossRecovery, F as FLOOR_MAX_FPS, a7 as announceWhenWhole, ak as drawSettledWhilePaused, aj as drawOnceWhilePaused, I as MessageEnvelope } from "./StationFloor-C8oG0qID.js";
import { dd as recipeKey, u as DEFAULT_CHARACTER, f as CAST_BY_NAME, cF as officeThemeBaseOf, df as recolorPixels, eb as useStore, dO as subscribeAvatars, b$ as isCustomAvatarId, dP as subscribeSeason, bl as castMemberFor, bQ as hexToNumber, by as currentSeason, bI as floorActionWord } from "./ui-CyUfM8WT.js";
import { resolveRecipe, costumeOf, sceneFrameBufs, SCENE_W, SCENE_H } from "./portraitArt-B3eZ2mvW.js";
import { a as applyLayoutToOfficeMap, o as officeSeatsFor, k as isStationPack, n as loadStationPack } from "./themeCatalog-CVKmJPh3.js";
import "./global-DYy7uepk.js";
import "./glyph-CD_xZXfA.js";
import "./puck-ac2UtXrw.js";
import "./lookArt-BobBg7Xf.js";
class AnimatedSprite extends Sprite {
  constructor(...args) {
    let options = args[0];
    if (Array.isArray(args[0])) {
      options = {
        textures: args[0],
        autoUpdate: args[1]
      };
    }
    const {
      animationSpeed = 1,
      autoPlay = false,
      autoUpdate = true,
      loop = true,
      onComplete = null,
      onFrameChange = null,
      onLoop = null,
      textures,
      updateAnchor = false,
      ...rest
    } = options;
    const [firstFrame] = textures;
    super({
      ...rest,
      texture: firstFrame instanceof Texture ? firstFrame : firstFrame.texture
    });
    this._textures = null;
    this._durations = null;
    this._autoUpdate = autoUpdate;
    this._isConnectedToTicker = false;
    this.animationSpeed = animationSpeed;
    this.loop = loop;
    this.updateAnchor = updateAnchor;
    this.onComplete = onComplete;
    this.onFrameChange = onFrameChange;
    this.onLoop = onLoop;
    this._currentTime = 0;
    this._playing = false;
    this._previousFrame = null;
    this.textures = textures;
    if (autoPlay) {
      this.play();
    }
  }
  /**
   * Stops the animation playback and freezes the current frame.
   * Does not reset the current frame or animation progress.
   * @example
   * ```ts
   * // Create an animated sprite
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('walk1.png'),
   *         Texture.from('walk2.png'),
   *         Texture.from('walk3.png')
   *     ],
   *     autoPlay: true
   * });
   *
   * // Stop at current frame
   * sprite.stop();
   *
   * // Stop at specific frame
   * sprite.gotoAndStop(1); // Stops at second frame
   *
   * // Stop and reset
   * sprite.stop();
   * sprite.currentFrame = 0;
   *
   * // Stop with completion check
   * if (sprite.playing) {
   *     sprite.stop();
   *     sprite.onComplete?.();
   * }
   * ```
   * @see {@link AnimatedSprite#play} For starting playback
   * @see {@link AnimatedSprite#gotoAndStop} For stopping at a specific frame
   * @see {@link AnimatedSprite#playing} For checking play state
   */
  stop() {
    if (!this._playing) {
      return;
    }
    this._playing = false;
    if (this._autoUpdate && this._isConnectedToTicker) {
      Ticker.shared.remove(this.update, this);
      this._isConnectedToTicker = false;
    }
  }
  /**
   * Starts or resumes the animation playback.
   * If the animation was previously stopped, it will continue from where it left off.
   * @example
   * ```ts
   * // Basic playback
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('walk1.png'),
   *         Texture.from('walk2.png'),
   *     ],
   *     autoPlay: false
   * });
   * sprite.play();
   *
   * // Play after stopping
   * sprite.stop();
   * sprite.currentFrame = 0; // Reset to start
   * sprite.play(); // Play from beginning
   *
   * // Play with auto-update disabled
   * sprite.autoUpdate = false;
   * sprite.play();
   * app.ticker.add(() => {
   *     sprite.update(app.ticker); // Manual updates
   * });
   * ```
   * @see {@link AnimatedSprite#stop} For stopping playback
   * @see {@link AnimatedSprite#gotoAndPlay} For playing from a specific frame
   * @see {@link AnimatedSprite#playing} For checking play state
   */
  play() {
    if (this._playing) {
      return;
    }
    this._playing = true;
    if (this._autoUpdate && !this._isConnectedToTicker) {
      Ticker.shared.add(this.update, this, UPDATE_PRIORITY.HIGH);
      this._isConnectedToTicker = true;
    }
  }
  /**
   * Stops the AnimatedSprite and sets it to a specific frame.
   * @example
   * ```ts
   * // Create an animated sprite
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('walk1.png'),
   *         Texture.from('walk2.png'),
   *         Texture.from('walk3.png'),
   *     ]
   * });
   *
   * // Go to specific frames
   * sprite.gotoAndStop(0);  // First frame
   * sprite.gotoAndStop(2);  // Third frame
   *
   * // Jump to last frame
   * sprite.gotoAndStop(sprite.totalFrames - 1);
   * ```
   * @param frameNumber - Frame index to stop at (0-based)
   * @throws {Error} If frameNumber is out of bounds
   * @see {@link AnimatedSprite#gotoAndPlay} For going to a frame and playing
   * @see {@link AnimatedSprite#currentFrame} For getting/setting current frame
   * @see {@link AnimatedSprite#totalFrames} For total number of frames
   */
  gotoAndStop(frameNumber) {
    this.stop();
    this.currentFrame = frameNumber;
  }
  /**
   * Goes to a specific frame and begins playing the AnimatedSprite from that point.
   * Combines frame navigation and playback start in one operation.
   * @example
   * ```ts
   * // Start from specific frame
   * sprite.gotoAndPlay(1); // Starts playing from second frame
   * ```
   * @param frameNumber - Frame index to start playing from (0-based)
   * @throws {Error} If frameNumber is out of bounds
   * @see {@link AnimatedSprite#gotoAndStop} For going to a frame without playing
   * @see {@link AnimatedSprite#play} For playing from current frame
   * @see {@link AnimatedSprite#currentFrame} For getting/setting current frame
   */
  gotoAndPlay(frameNumber) {
    this.currentFrame = frameNumber;
    this.play();
  }
  /**
   * Updates the object transform for rendering. This method handles animation timing, frame updates,
   * and manages looping behavior.
   * @example
   * ```ts
   * // Create an animated sprite with manual updates
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('frame1.png'),
   *         Texture.from('frame2.png'),
   *         Texture.from('frame3.png')
   *     ],
   *     autoUpdate: false // Disable automatic updates
   * });
   *
   * // Manual update with app ticker
   * app.ticker.add((ticker) => {
   *     sprite.update(ticker);
   * });
   * ```
   * @param ticker - The ticker to use for updating the animation timing
   * @see {@link AnimatedSprite#autoUpdate} For controlling automatic updates
   * @see {@link AnimatedSprite#animationSpeed} For controlling animation speed
   * @see {@link Ticker} For timing system details
   */
  update(ticker) {
    if (!this._playing) {
      return;
    }
    const deltaTime = ticker.deltaTime;
    const elapsed = this.animationSpeed * deltaTime;
    const previousFrame = this.currentFrame;
    if (this._durations !== null) {
      let lag = this._currentTime % 1 * this._durations[this.currentFrame];
      lag += elapsed / 60 * 1e3;
      while (lag < 0) {
        this._currentTime--;
        lag += this._durations[this.currentFrame];
      }
      const sign = Math.sign(this.animationSpeed * deltaTime);
      this._currentTime = Math.floor(this._currentTime);
      while (lag >= this._durations[this.currentFrame]) {
        lag -= this._durations[this.currentFrame] * sign;
        this._currentTime += sign;
      }
      this._currentTime += lag / this._durations[this.currentFrame];
    } else {
      this._currentTime += elapsed;
    }
    if (this._currentTime < 0 && !this.loop) {
      this.gotoAndStop(0);
      if (this.onComplete) {
        this.onComplete();
      }
    } else if (this._currentTime >= this._textures.length && !this.loop) {
      this.gotoAndStop(this._textures.length - 1);
      if (this.onComplete) {
        this.onComplete();
      }
    } else if (previousFrame !== this.currentFrame) {
      if (this.loop && this.onLoop) {
        if (this.animationSpeed > 0 && this.currentFrame < previousFrame || this.animationSpeed < 0 && this.currentFrame > previousFrame) {
          this.onLoop();
        }
      }
      this._updateTexture();
    }
  }
  /** Updates the displayed texture to match the current frame index. */
  _updateTexture() {
    const currentFrame = this.currentFrame;
    if (this._previousFrame === currentFrame) {
      return;
    }
    this._previousFrame = currentFrame;
    this.texture = this._textures[currentFrame];
    if (this.updateAnchor && this.texture.defaultAnchor) {
      this.anchor.copyFrom(this.texture.defaultAnchor);
    }
    if (this.onFrameChange) {
      this.onFrameChange(this.currentFrame);
    }
  }
  /**
   * Stops the AnimatedSprite and destroys it.
   * This method stops the animation playback, removes it from the ticker,
   * and cleans up any resources associated with the sprite.
   * @param options - Options for destroying the sprite, such as whether to remove from parent
   * @example
   * ```ts
   * // Destroy the sprite when done
   * sprite.destroy();
   * // Or with options
   * sprite.destroy({ children: true, texture: true, textureSource: true });
   * ```
   */
  destroy(options = false) {
    const destroyTexture = typeof options === "boolean" ? options : options?.texture;
    if (destroyTexture) {
      const destroyTextureSource = typeof options === "boolean" ? options : options?.textureSource;
      this._textures.forEach((texture) => {
        if (this.texture !== texture) {
          texture.destroy(destroyTextureSource);
        }
      });
    }
    this._textures = [];
    this._durations = null;
    this.stop();
    super.destroy(options);
    this.onComplete = null;
    this.onFrameChange = null;
    this.onLoop = null;
  }
  /**
   * A short hand way of creating an AnimatedSprite from an array of frame ids.
   * Uses texture frames from the cache to create an animation sequence.
   * @example
   * ```ts
   * // Create from frame IDs
   * const frameIds = [
   *     'walk_001.png',
   *     'walk_002.png',
   *     'walk_003.png'
   * ];
   *
   * const walkingAnimation = AnimatedSprite.fromFrames(frameIds);
   * walkingAnimation.play();
   * ```
   * @param frames - The array of frame ids to use for the animation
   * @returns A new animated sprite using the frames
   * @see {@link Texture.from} For texture creation from frames
   * @see {@link Spritesheet} For loading spritesheets
   */
  static fromFrames(frames) {
    const textures = [];
    for (let i = 0; i < frames.length; ++i) {
      textures.push(Texture.from(frames[i]));
    }
    return new AnimatedSprite(textures);
  }
  /**
   * A short hand way of creating an AnimatedSprite from an array of image urls.
   * Each image will be used as a frame in the animation.
   * @example
   * ```ts
   * // Create from image URLs
   * const images = [
   *     'assets/walk1.png',
   *     'assets/walk2.png',
   *     'assets/walk3.png'
   * ];
   *
   * const walkingSprite = AnimatedSprite.fromImages(images);
   * walkingSprite.play();
   * ```
   * @param images - The array of image urls to use as frames
   * @returns A new animated sprite using the images as frames
   * @see {@link Assets} For asset loading and management
   * @see {@link Texture.from} For texture creation from images
   */
  static fromImages(images) {
    const textures = [];
    for (let i = 0; i < images.length; ++i) {
      textures.push(Texture.from(images[i]));
    }
    return new AnimatedSprite(textures);
  }
  /**
   * The total number of frames in the AnimatedSprite. This is the same as number of textures
   * assigned to the AnimatedSprite.
   * @example
   * ```ts
   * // Create an animated sprite
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('frame1.png'),
   *         Texture.from('frame2.png'),
   *         Texture.from('frame3.png')
   *     ]
   * });
   *
   * // Get total frames
   * console.log(sprite.totalFrames); // Outputs: 3
   *
   * // Use with frame navigation
   * sprite.gotoAndStop(sprite.totalFrames - 1); // Go to last frame
   * ```
   * @readonly
   * @see {@link AnimatedSprite#currentFrame} For the current frame index
   * @see {@link AnimatedSprite#textures} For the array of textures
   * @returns {number} The total number of frames
   */
  get totalFrames() {
    return this._textures.length;
  }
  /**
   * The array of textures or frame objects used for the animation sequence.
   * Can be set to either an array of Textures or an array of FrameObjects with custom timing.
   * @example
   * ```ts
   * // Update textures at runtime
   * sprite.textures = [
   *     Texture.from('run1.png'),
   *     Texture.from('run2.png')
   * ];
   *
   * // Use custom frame timing
   * sprite.textures = [
   *     { texture: Texture.from('explosion1.png'), time: 100 },
   *     { texture: Texture.from('explosion2.png'), time: 200 },
   *     { texture: Texture.from('explosion3.png'), time: 300 }
   * ];
   *
   * // Use with spritesheet
   * const sheet = await Assets.load('animations.json');
   * sprite.textures = sheet.animations['walk'];
   * ```
   * @type {AnimatedSpriteFrames}
   * @see {@link FrameObject} For frame timing options
   * @see {@link Spritesheet} For loading from spritesheets
   */
  get textures() {
    return this._textures;
  }
  set textures(value) {
    if (value[0] instanceof Texture) {
      this._textures = value;
      this._durations = null;
    } else {
      this._textures = [];
      this._durations = [];
      for (let i = 0; i < value.length; i++) {
        this._textures.push(value[i].texture);
        this._durations.push(value[i].time);
      }
    }
    this._previousFrame = null;
    this.gotoAndStop(0);
    this._updateTexture();
  }
  /**
   * Gets or sets the current frame index of the animation.
   * When setting, the value will be clamped between 0 and totalFrames - 1.
   * @example
   * ```ts
   * // Create an animated sprite
   * const sprite = new AnimatedSprite({
   *     textures: [
   *         Texture.from('walk1.png'),
   *         Texture.from('walk2.png'),
   *         Texture.from('walk3.png')
   *     ]
   * });
   *
   * // Get current frame
   * console.log(sprite.currentFrame); // 0
   *
   * // Set specific frame
   * sprite.currentFrame = 1; // Show second frame
   *
   * // Use with frame callbacks
   * sprite.onFrameChange = (frame) => {
   *     console.log(`Now showing frame: ${frame}`);
   * };
   * sprite.currentFrame = 2;
   * ```
   * @throws {Error} If attempting to set a frame index out of bounds
   * @see {@link AnimatedSprite#totalFrames} For the total number of frames
   * @see {@link AnimatedSprite#gotoAndPlay} For playing from a specific frame
   * @see {@link AnimatedSprite#gotoAndStop} For stopping at a specific frame
   */
  get currentFrame() {
    let currentFrame = Math.floor(this._currentTime) % this._textures.length;
    if (currentFrame < 0) {
      currentFrame += this._textures.length;
    }
    return currentFrame;
  }
  set currentFrame(value) {
    if (value < 0 || value > this.totalFrames - 1) {
      throw new Error(`[AnimatedSprite]: Invalid frame index value ${value}, expected to be between 0 and totalFrames ${this.totalFrames}.`);
    }
    const previousFrame = this.currentFrame;
    this._currentTime = value;
    if (previousFrame !== this.currentFrame) {
      this._updateTexture();
    }
  }
  /**
   * Indicates if the AnimatedSprite is currently playing.
   * This is a read-only property that reflects the current playback state.
   * @example
   * ```ts
   * // Check if animation is playing
   * console.log('Playing:', sprite.playing); // true
   *
   * // Use with play control
   * if (!sprite.playing) {
   *     sprite.play();
   * }
   * ```
   * @readonly
   * @returns {boolean} True if the animation is currently playing
   * @see {@link AnimatedSprite#play} For starting playback
   * @see {@link AnimatedSprite#stop} For stopping playback
   * @see {@link AnimatedSprite#loop} For controlling looping behavior
   */
  get playing() {
    return this._playing;
  }
  /**
   * Controls whether the animation automatically updates using the shared ticker.
   * When enabled, the animation will update on each frame. When disabled, you must
   * manually call update() to advance the animation.
   * @example
   * ```ts
   * // Create sprite with auto-update disabled
   * const sprite = new AnimatedSprite({
   *     textures: [],
   *     autoUpdate: false
   * });
   *
   * // Manual update with app ticker
   * app.ticker.add((ticker) => {
   *     sprite.update(ticker);
   * });
   *
   * // Enable auto-update later
   * sprite.autoUpdate = true;
   * ```
   * @default true
   * @see {@link AnimatedSprite#update} For manual animation updates
   * @see {@link Ticker} For the timing system
   */
  get autoUpdate() {
    return this._autoUpdate;
  }
  set autoUpdate(value) {
    if (value !== this._autoUpdate) {
      this._autoUpdate = value;
      if (!this._autoUpdate && this._isConnectedToTicker) {
        Ticker.shared.remove(this.update, this);
        this._isConnectedToTicker = false;
      } else if (this._autoUpdate && !this._isConnectedToTicker && this._playing) {
        Ticker.shared.add(this.update, this);
        this._isConnectedToTicker = true;
      }
    }
  }
}
const FLIPPED_H_FLAG = 2147483648;
const FLIPPED_V_FLAG = 1073741824;
const FLIPPED_D_FLAG = 536870912;
const TILE_ID_MASK = 536870911;
const TILE_LAYERS = ["floor", "walls", "furniture-below", "furniture-above"];
const COLLISION_LAYER = "collision";
const SPAWN_POINTS_LAYER = "spawn-points";
const ZONES_LAYER = "zones";
class TiledMapRenderer {
  constructor(mapData, tilesetTextures) {
    this.mapData = mapData;
    this.tilesetTextures = tilesetTextures;
    this.width = mapData.width;
    this.height = mapData.height;
    this.tileSize = mapData.tilewidth;
    this.rootContainer = new Container();
    this.characterContainer = new Container();
    this.characterContainer.sortableChildren = true;
    this.parseCollisionLayer();
    this.parseSpawnPoints();
    this.markWalkableSpawnPoints();
    this.parseZones();
    this.buildTileLayers();
  }
  width;
  height;
  tileSize;
  walkabilityGrid = [];
  spawnPoints = /* @__PURE__ */ new Map();
  zones = /* @__PURE__ */ new Map();
  characterContainer;
  rootContainer;
  static WALKABLE_SPAWN_PREFIXES = ["desk-", "pc-", "warroom-", "entrance"];
  getContainer() {
    return this.rootContainer;
  }
  getCharacterContainer() {
    return this.characterContainer;
  }
  isWalkable(tx, ty) {
    if (tx < 0 || ty < 0 || tx >= this.width || ty >= this.height) return false;
    return this.walkabilityGrid[ty][tx];
  }
  tileToPixel(tx, ty) {
    return { x: tx * this.tileSize, y: ty * this.tileSize };
  }
  pixelToTile(px, py) {
    return { x: Math.floor(px / this.tileSize), y: Math.floor(py / this.tileSize) };
  }
  getSpawnPoint(name) {
    return this.spawnPoints.get(name);
  }
  getAllSpawnPoints() {
    return this.spawnPoints;
  }
  getZone(name) {
    return this.zones.get(name);
  }
  getAllZones() {
    return this.zones;
  }
  /** The (flip-stripped) gid painted at a tile of a layer, 0 when empty.
   *  Lets the scene locate furniture by art — e.g. each desk's monitor block. */
  gidAt(layerName, tx, ty) {
    const layer = this.findLayer(layerName, "tilelayer");
    if (!layer?.data || tx < 0 || ty < 0 || tx >= this.width || ty >= this.height) return 0;
    return (layer.data[ty * this.width + tx] ?? 0) & TILE_ID_MASK;
  }
  /** A sub-texture for one tileset tile, addressed by gid — for dynamic props
   *  that reuse map art (e.g. the lit monitor variant overlaid when an agent
   *  sits down). Returns undefined for gid 0 / out-of-range.
   *  NOTE: allocates a fresh Texture per call and the CALLER owns its lifetime
   *  (fine for constructor-time use, kept alive by sprites; do NOT call this
   *  per frame or the textures leak). */
  textureForGid(gid) {
    const tileId = gid & TILE_ID_MASK;
    if (tileId === 0) return void 0;
    const resolved = this.resolveTileset(tileId);
    if (!resolved) return void 0;
    const { tileset, texture } = resolved;
    const cols = tileset.columns ?? 16;
    const tw = tileset.tilewidth ?? this.tileSize;
    const th = tileset.tileheight ?? this.tileSize;
    const localId = tileId - tileset.firstgid;
    const frame = new Rectangle(localId % cols * tw, Math.floor(localId / cols) * th, tw, th);
    return new Texture({ source: texture.source, frame });
  }
  parseCollisionLayer() {
    const layer = this.findLayer(COLLISION_LAYER, "tilelayer");
    this.walkabilityGrid = Array.from({ length: this.height }, () => Array(this.width).fill(true));
    if (!layer?.data) return;
    for (let y = 0; y < this.height; y++) {
      for (let x = 0; x < this.width; x++) {
        const rawId = layer.data[y * this.width + x];
        if ((rawId & TILE_ID_MASK) !== 0) this.walkabilityGrid[y][x] = false;
      }
    }
  }
  parseSpawnPoints() {
    const layer = this.findLayer(SPAWN_POINTS_LAYER, "objectgroup");
    if (!layer?.objects) return;
    for (const obj of layer.objects) {
      this.spawnPoints.set(obj.name, {
        x: Math.floor(obj.x / this.tileSize),
        y: Math.floor(obj.y / this.tileSize)
      });
    }
  }
  /** Force seat/desk tiles walkable so agents can path onto them even though
   *  the underlying chair/desk tile is non-walkable in the collision layer. */
  markWalkableSpawnPoints() {
    for (const [name, point] of this.spawnPoints) {
      if (!TiledMapRenderer.WALKABLE_SPAWN_PREFIXES.some((p) => name.startsWith(p))) continue;
      if (point.y >= 0 && point.y < this.height && point.x >= 0 && point.x < this.width) {
        this.walkabilityGrid[point.y][point.x] = true;
      }
    }
  }
  parseZones() {
    const layer = this.findLayer(ZONES_LAYER, "objectgroup");
    if (!layer?.objects) return;
    for (const obj of layer.objects) {
      this.zones.set(obj.name, {
        x: Math.floor(obj.x / this.tileSize),
        y: Math.floor(obj.y / this.tileSize),
        width: Math.floor((obj.width ?? 0) / this.tileSize),
        height: Math.floor((obj.height ?? 0) / this.tileSize)
      });
    }
  }
  resolveTileset(tileId) {
    for (let i = this.mapData.tilesets.length - 1; i >= 0; i--) {
      if (tileId >= this.mapData.tilesets[i].firstgid) {
        return { tileset: this.mapData.tilesets[i], texture: this.tilesetTextures[i] };
      }
    }
    return void 0;
  }
  buildTileLayers() {
    if (this.mapData.tilesets.length === 0) return;
    for (const layerName of TILE_LAYERS) {
      const layer = this.findLayer(layerName, "tilelayer");
      const container = new Container();
      container.label = layerName;
      if (layer?.data) {
        for (let y = 0; y < this.height; y++) {
          for (let x = 0; x < this.width; x++) {
            const raw = layer.data[y * this.width + x];
            if (raw === 0) continue;
            const flippedH = (raw & FLIPPED_H_FLAG) !== 0;
            const flippedV = (raw & FLIPPED_V_FLAG) !== 0;
            const flippedD = (raw & FLIPPED_D_FLAG) !== 0;
            const tileId = raw & TILE_ID_MASK;
            const resolved = this.resolveTileset(tileId);
            if (!resolved) continue;
            const { tileset, texture } = resolved;
            const cols = tileset.columns ?? 16;
            const tw = tileset.tilewidth ?? this.tileSize;
            const th = tileset.tileheight ?? this.tileSize;
            const localId = tileId - tileset.firstgid;
            const srcX = localId % cols * tw;
            const srcY = Math.floor(localId / cols) * th;
            const frame = new Rectangle(srcX, srcY, tw, th);
            const sprite = new Sprite(new Texture({ source: texture.source, frame }));
            if (flippedH || flippedV || flippedD) {
              sprite.anchor.set(0.5, 0.5);
              sprite.x = x * this.tileSize + this.tileSize / 2;
              sprite.y = y * this.tileSize + this.tileSize / 2;
              if (flippedD) {
                if (flippedH && !flippedV) {
                  sprite.rotation = Math.PI / 2;
                } else if (!flippedH && flippedV) {
                  sprite.rotation = -Math.PI / 2;
                } else if (flippedH && flippedV) {
                  sprite.rotation = Math.PI / 2;
                  sprite.scale.y = -1;
                } else {
                  sprite.rotation = Math.PI / 2;
                  sprite.scale.x = -1;
                }
              } else {
                if (flippedH) sprite.scale.x = -1;
                if (flippedV) sprite.scale.y = -1;
              }
            } else {
              sprite.x = x * this.tileSize;
              sprite.y = y * this.tileSize;
            }
            container.addChild(sprite);
          }
        }
      }
      this.rootContainer.addChild(container);
    }
    this.rootContainer.addChild(this.characterContainer);
  }
  findLayer(name, type) {
    return this.mapData.layers.find((l) => l.name === name && l.type === type);
  }
}
function timeScaledLerp(perFrame, dt, refHz = 60) {
  if (!(dt > 0)) return 0;
  return 1 - Math.pow(1 - perFrame, dt * refHz);
}
const LERP_SPEED = 0.08;
const REF_HZ = 60;
class Camera {
  container;
  currentX = 0;
  currentY = 0;
  currentZoom = 1;
  targetX = 0;
  targetY = 0;
  targetZoom = 1;
  viewWidth = 960;
  viewHeight = 800;
  mapWidth = 640;
  mapHeight = 480;
  // M30-261: the part of the map that holds art; fitted and centred. The whole map unless setArt says less.
  artX = 0;
  artY = 0;
  artW = 640;
  artH = 480;
  manualOverride = false;
  nudgeOffsetX = 0;
  nudgeOffsetY = 0;
  nudgeElapsed = 0;
  nudgeDuration = 0;
  nudgeStrength = 0.4;
  constructor(container) {
    this.container = container;
  }
  setMapSize(width, height) {
    this.mapWidth = width;
    this.mapHeight = height;
    this.artX = 0;
    this.artY = 0;
    this.artW = width;
    this.artH = height;
  }
  /** The art's box in map px (after setMapSize); clamped to the map. */
  setArt(x, y, w, h) {
    this.artX = Math.max(0, x);
    this.artY = Math.max(0, y);
    this.artW = Math.min(w, this.mapWidth - this.artX);
    this.artH = Math.min(h, this.mapHeight - this.artY);
  }
  setViewSize(width, height) {
    this.viewWidth = width;
    this.viewHeight = height;
    if (!this.manualOverride) this.fitToScreen();
  }
  getMinZoom() {
    if (this.viewWidth === 0 || this.viewHeight === 0) return 1;
    return Math.min(this.viewWidth / this.artW, this.viewHeight / this.artH);
  }
  /** Fit the whole art to the viewport, centered. */
  fitToScreen() {
    this.manualOverride = false;
    this.targetX = this.artX + this.artW / 2;
    this.targetY = this.artY + this.artH / 2;
    this.targetZoom = this.getMinZoom();
  }
  /** Pan/zoom toward a world point (used when an agent is selected). */
  focusOn(worldX, worldY, zoom) {
    this.manualOverride = true;
    this.targetX = worldX;
    this.targetY = worldY;
    this.targetZoom = Math.max(this.getMinZoom(), Math.min(4, zoom ?? this.currentZoom));
  }
  /** A gentle, decaying pan toward a world point without taking manual control. */
  nudgeToward(worldX, worldY, duration = 1200) {
    if (this.manualOverride) return;
    this.nudgeOffsetX = (worldX - this.targetX) * this.nudgeStrength;
    this.nudgeOffsetY = (worldY - this.targetY) * this.nudgeStrength;
    this.nudgeElapsed = 0;
    this.nudgeDuration = duration / 1e3;
  }
  /** Be where the camera is going, at once: a paused floor draws one frame, and the 1.6 s ease of a new fit
   *  would leave that frame half way, zoomed out on a dark field (Kevin, b6j). */
  snapToTarget() {
    this.currentX = this.targetX;
    this.currentY = this.targetY;
    this.currentZoom = this.targetZoom;
    this.nudgeDuration = 0;
    this.nudgeOffsetX = 0;
    this.nudgeOffsetY = 0;
  }
  update(dt) {
    const k = timeScaledLerp(LERP_SPEED, dt, REF_HZ);
    this.currentX += (this.targetX - this.currentX) * k;
    this.currentY += (this.targetY - this.currentY) * k;
    this.currentZoom += (this.targetZoom - this.currentZoom) * k;
    if (this.nudgeDuration > 0) {
      this.nudgeElapsed += dt;
      const t = Math.min(this.nudgeElapsed / this.nudgeDuration, 1);
      const ease = (1 - t) * dt * REF_HZ;
      this.currentX += this.nudgeOffsetX * ease;
      this.currentY += this.nudgeOffsetY * ease;
      if (t >= 1) {
        this.nudgeDuration = 0;
        this.nudgeOffsetX = 0;
        this.nudgeOffsetY = 0;
      }
    }
    this.container.scale.set(this.currentZoom);
    this.container.x = this.viewWidth / 2 - this.currentX * this.currentZoom;
    this.container.y = this.viewHeight / 2 - this.currentY * this.currentZoom;
    const z = this.currentZoom;
    const scaledW = this.artW * z;
    const scaledH = this.artH * z;
    if (scaledW <= this.viewWidth) {
      this.container.x = (this.viewWidth - scaledW) / 2 - this.artX * z;
    } else {
      this.container.x = Math.min(-this.artX * z, Math.max(this.viewWidth - (this.artX + this.artW) * z, this.container.x));
    }
    if (scaledH <= this.viewHeight) {
      this.container.y = (this.viewHeight - scaledH) / 2 - this.artY * z;
    } else {
      this.container.y = Math.min(-this.artY * z, Math.max(this.viewHeight - (this.artY + this.artH) * z, this.container.y));
    }
  }
}
const DIRECTION_ROW = {
  down: 0,
  up: 1,
  right: 2,
  left: 2
};
const ANIM_FRAMES = {
  walk: [0, 1, 2, 1],
  type: [0, 1, 2, 1],
  read: [0, 1, 2, 1],
  idle: [0]
};
const CHAR_SCALE = 1.08;
class CharacterSprite {
  container;
  sprite;
  frames;
  currentDirection = "down";
  currentAnim = "idle";
  frameSpeed = 0.15;
  frameW;
  frameH;
  cropMask = null;
  constructor(frames) {
    this.frames = frames;
    this.container = new Container();
    const initialFrames = this.getFrames("down", "idle");
    this.sprite = new AnimatedSprite(initialFrames);
    this.sprite.anchor.set(0.5, 1);
    this.sprite.animationSpeed = this.frameSpeed;
    this.sprite.autoUpdate = false;
    this.sprite.play();
    this.frameW = this.sprite.texture.frame.width || this.sprite.width || 16;
    this.frameH = this.sprite.texture.frame.height || this.sprite.height || 32;
    this.container.addChild(this.sprite);
    this.container.scale.set(CHAR_SCALE);
  }
  /**
   * Crop `cropPx` off the bottom of the sprite (the legs) so a seated agent
   * reads as tucked under the desk instead of standing on top of it. Pass 0 to
   * clear the crop (standing / walking). The mask only covers the sprite, so
   * status glyphs / bubbles parented elsewhere are unaffected.
   */
  setSeatedCrop(cropPx) {
    if (cropPx <= 0) {
      if (this.cropMask) {
        this.sprite.mask = null;
        this.cropMask.visible = false;
      }
      return;
    }
    if (!this.cropMask) {
      this.cropMask = new Graphics();
      this.container.addChild(this.cropMask);
    }
    const w = this.frameW;
    const h = this.frameH;
    this.cropMask.clear();
    this.cropMask.rect(-w / 2 - 2, -h - 2, w + 4, h - cropPx + 2).fill(16777215);
    this.cropMask.visible = true;
    this.sprite.mask = this.cropMask;
  }
  /** Swap the whole frame grid (an edited custom avatar). Direction, animation
   *  and the seat crop are kept; the sprite keeps playing on the new textures. */
  replaceFrames(frames) {
    this.frames = frames;
    this.sprite.textures = this.getFrames(this.currentDirection, this.currentAnim);
    this.sprite.play();
  }
  /** Advance the walk/idle cycle by `dt` seconds.
   *
   *  AnimatedSprite.update() reads exactly one field off what it is handed —
   *  `deltaTime`, in 60 Hz frames — so a reused scratch object drives it without
   *  allocating per character per frame or owning a second Ticker. */
  advance(dt) {
    CharacterSprite.beat.deltaTime = dt * 60;
    this.sprite.update(CharacterSprite.beat);
  }
  static beat = { deltaTime: 0 };
  getFrames(direction, anim) {
    const row = DIRECTION_ROW[direction];
    return ANIM_FRAMES[anim].map((col) => this.frames[row][col]);
  }
  setAnimation(anim, direction) {
    if (anim === this.currentAnim && direction === this.currentDirection) return;
    this.currentAnim = anim;
    this.currentDirection = direction;
    this.sprite.textures = this.getFrames(direction, anim);
    this.sprite.scale.x = direction === "left" ? -1 : 1;
    this.sprite.animationSpeed = anim === "walk" ? 0.15 : anim === "idle" ? 0.08 : 0.06;
    this.sprite.play();
  }
  setPosition(x, y) {
    this.container.x = x;
    this.container.y = y;
  }
  setAlpha(alpha) {
    this.container.alpha = alpha;
  }
  destroy() {
    this.container.destroy({ children: true });
  }
}
const DIRECTIONS = [
  { x: 0, y: -1 },
  { x: 0, y: 1 },
  { x: -1, y: 0 },
  { x: 1, y: 0 }
];
function findPath(map, start, goal) {
  if (start.x === goal.x && start.y === goal.y) return [];
  if (!map.isWalkable(goal.x, goal.y)) return null;
  const key = (p) => `${p.x},${p.y}`;
  const visited = /* @__PURE__ */ new Set();
  const parent = /* @__PURE__ */ new Map();
  const queue = [start];
  visited.add(key(start));
  while (queue.length > 0) {
    const current = queue.shift();
    for (const dir of DIRECTIONS) {
      const next = { x: current.x + dir.x, y: current.y + dir.y };
      const nextKey = key(next);
      if (visited.has(nextKey) || !map.isWalkable(next.x, next.y)) continue;
      visited.add(nextKey);
      parent.set(nextKey, current);
      if (next.x === goal.x && next.y === goal.y) {
        return reconstructPath(parent, start, goal);
      }
      queue.push(next);
    }
  }
  return null;
}
function reconstructPath(parent, start, goal) {
  const path = [];
  let current = goal;
  const key = (p) => `${p.x},${p.y}`;
  while (!(current.x === start.x && current.y === start.y)) {
    path.unshift(current);
    current = parent.get(key(current));
  }
  return path;
}
function advanceAlongPath(x, y, path, toPixel, budget) {
  let remaining = Math.max(0, budget);
  let consumed = 0;
  for (const waypoint of path) {
    const target = toPixel(waypoint);
    const dx = target.x - x;
    const dy = target.y - y;
    const dist = Math.sqrt(dx * dx + dy * dy);
    if (dist <= remaining) {
      x = target.x;
      y = target.y;
      remaining -= dist;
      consumed++;
      continue;
    }
    x += dx / dist * remaining;
    y += dy / dist * remaining;
    remaining = 0;
    break;
  }
  return { x, y, consumed, leftover: remaining };
}
const PADDING_X = 6;
const PADDING_Y = 3;
const CORNER_RADIUS = 5;
const MAX_WIDTH = 150;
const FILL_COLOR = colors.cream[50];
const OUTLINE_COLOR = colors.ink[900];
const TEXT_COLOR = "#3d2e4a";
const FONT_SIZE = 12;
const RENDER_SCALE = 0.5;
const OFFSET_Y = -38;
const FADE_IN_DURATION = 0.15;
const FADE_OUT_DURATION = 0.3;
const LINGER_DURATION = 1.2;
const DOTS_CYCLE_SPEED = 0.45;
const WRAP_WIDTH = MAX_WIDTH / RENDER_SCALE - PADDING_X * 2;
const MAX_CHARS = 160;
class ThoughtBubble {
  container;
  inner;
  bg;
  tail;
  label;
  state = "hidden";
  fadeElapsed = 0;
  lingerElapsed = 0;
  bgW = 0;
  bgH = 0;
  isThinking = false;
  dotsElapsed = 0;
  dotsPhase = 0;
  // Extra upward shift (px) applied on top of OFFSET_Y so two nearby bubbles can
  // stack instead of overlapping. Set each frame by the scene's overlap pass.
  extraLift = 0;
  // Current camera zoom. The bubble lives inside the zoomed world container, so
  // when the window shrinks (fit-to-screen zoom < 1) the text used to scale
  // down with the map and turn into ragged mush. Counter-scale so the bubble
  // never renders below its designed 1:1 screen size; at zoom ≥ 1 it keeps
  // scaling with the world as before.
  zoom = 1;
  // World bounds (map size in px). An avatar near the map edge — Michael's CEO
  // room sits in the top-left corner — would otherwise push its cloud out of
  // the visible world. setPosition clamps the rect back inside, tooltip-style.
  boundsW = 0;
  boundsH = 0;
  boundsX = 0;
  boundsY = 0;
  constructor() {
    this.container = new Container();
    this.container.zIndex = 1e5;
    this.container.eventMode = "none";
    this.container.alpha = 0;
    this.container.visible = false;
    this.inner = new Container();
    this.inner.scale.set(RENDER_SCALE);
    this.container.addChild(this.inner);
    this.tail = new Graphics();
    this.bg = new Graphics();
    this.label = new Text({
      text: "",
      style: {
        fontSize: FONT_SIZE,
        fontWeight: "bold",
        fill: TEXT_COLOR,
        fontFamily: "monospace",
        align: "left",
        wordWrap: true,
        wordWrapWidth: WRAP_WIDTH,
        breakWords: true
      }
    });
    this.label.x = PADDING_X;
    this.label.y = PADDING_Y;
    this.inner.addChild(this.tail, this.bg, this.label);
  }
  /** Show the current activity. Empty text → an animated "…" (model thinking).
   *  `tool` (an agent's `carrying`) prefixes a small glyph when present. */
  show(text, tool) {
    this.isThinking = !text.trim();
    if (this.isThinking) {
      this.dotsElapsed = 0;
      this.dotsPhase = 0;
      this.label.text = ".";
    } else {
      const display = tool ? `${toolIcon(tool)} ${text}` : text;
      this.label.text = display.length > MAX_CHARS ? display.slice(0, MAX_CHARS - 1).trimEnd() + "…" : display;
    }
    this.redraw();
    this.reveal();
  }
  reveal() {
    if (this.state === "hidden" || this.state === "fading-out") {
      this.state = "fading-in";
      this.fadeElapsed = 0;
      this.container.visible = true;
    } else {
      this.state = "visible";
      this.container.alpha = 1;
    }
    this.lingerElapsed = 0;
  }
  /** Begin fading out (after a short linger) — call when the agent goes quiet. */
  startLinger() {
    if (this.state === "hidden") return;
    this.state = "lingering";
    this.lingerElapsed = 0;
  }
  /** Update for the camera zoom: keep the bubble's SCREEN size from dropping
   *  below 1:1 by counter-scaling while the world is zoomed out. */
  setZoom(z) {
    if (!(z > 0) || z === this.zoom) return;
    this.zoom = z;
    this.container.scale.set(this.compensation());
  }
  /** World-units multiplier that cancels a < 1 camera zoom (1 at zoom ≥ 1). */
  compensation() {
    return 1 / Math.min(this.zoom, 1);
  }
  /** The world rect the bubble must stay inside, in map px: the map, or the part shown inside the edge (M30-261). */
  setBounds(w, h, x = 0, y = 0) {
    this.boundsW = w;
    this.boundsH = h;
    this.boundsX = x;
    this.boundsY = y;
  }
  setPosition(px, py) {
    const comp = this.compensation();
    const w = this.bgW * RENDER_SCALE * comp;
    const h = this.bgH * RENDER_SCALE * comp;
    let x = px - w / 2;
    let y = py + OFFSET_Y - h - this.extraLift;
    if (this.boundsW > 0) {
      const x0 = this.boundsX + 1, y0 = this.boundsY + 1;
      x = Math.min(Math.max(x, x0), Math.max(x0, this.boundsX + this.boundsW - w - 1));
      y = Math.min(Math.max(y, y0), Math.max(y0, this.boundsY + this.boundsH - h - 1));
    }
    this.container.x = Math.round(x);
    this.container.y = Math.round(y);
  }
  /** Extra upward shift (px), set by the scene's bubble-overlap pass. */
  setLift(px) {
    this.extraLift = px;
  }
  /** The bubble's base screen rect (ignoring any lift) for a given anchor, or
   *  null when hidden. Used by the scene to detect and resolve overlaps. */
  getLayout(px, py) {
    if (this.state === "hidden") return null;
    const comp = this.compensation();
    const w = this.bgW * RENDER_SCALE * comp;
    const h = this.bgH * RENDER_SCALE * comp;
    let x = px - w / 2;
    let y = py + OFFSET_Y - h;
    if (this.boundsW > 0) {
      const x0 = this.boundsX + 1, y0 = this.boundsY + 1;
      x = Math.min(Math.max(x, x0), Math.max(x0, this.boundsX + this.boundsW - w - 1));
      y = Math.min(Math.max(y, y0), Math.max(y0, this.boundsY + this.boundsH - h - 1));
    }
    return { x, y, w, h };
  }
  hide() {
    this.state = "hidden";
    this.isThinking = false;
    this.container.alpha = 0;
    this.container.visible = false;
  }
  isHidden() {
    return this.state === "hidden";
  }
  update(dt) {
    if (this.isThinking && (this.state === "visible" || this.state === "fading-in")) {
      this.dotsElapsed += dt;
      const newPhase = Math.floor(this.dotsElapsed / DOTS_CYCLE_SPEED) % 3;
      if (newPhase !== this.dotsPhase) {
        this.dotsPhase = newPhase;
        this.label.text = [".", "..", "..."][this.dotsPhase];
        this.redraw();
      }
    }
    switch (this.state) {
      case "fading-in": {
        this.fadeElapsed += dt;
        const t = Math.min(this.fadeElapsed / FADE_IN_DURATION, 1);
        this.container.alpha = t;
        if (t >= 1) this.state = "visible";
        break;
      }
      case "lingering": {
        this.lingerElapsed += dt;
        if (this.lingerElapsed >= LINGER_DURATION) {
          this.state = "fading-out";
          this.fadeElapsed = 0;
        }
        break;
      }
      case "fading-out": {
        this.fadeElapsed += dt;
        const t = Math.min(this.fadeElapsed / FADE_OUT_DURATION, 1);
        this.container.alpha = 1 - t;
        if (t >= 1) this.hide();
        break;
      }
    }
  }
  destroy() {
    this.container.destroy({ children: true });
  }
  redraw() {
    this.bgW = this.label.width + PADDING_X * 2;
    this.bgH = this.label.height + PADDING_Y * 2;
    this.bg.clear();
    this.bg.roundRect(0, 0, this.bgW, this.bgH, CORNER_RADIUS);
    this.bg.fill({ color: FILL_COLOR });
    this.bg.stroke({ color: OUTLINE_COLOR, width: 1 });
    this.tail.clear();
    const baseX = this.bgW * 0.32;
    const puff = (cx, cy, r) => {
      this.tail.circle(cx, cy, r).fill({ color: FILL_COLOR }).stroke({ color: OUTLINE_COLOR, width: 1 });
    };
    puff(baseX, this.bgH + 4, 3);
    puff(baseX - 5, this.bgH + 9, 2);
  }
}
function lerp(a, b, t) {
  const tt = Math.min(Math.max(t, 0), 1);
  return a + (b - a) * tt;
}
function paintCup(g, x, y) {
  g.rect(x, y - 4, 5, 4).fill(15920610);
  g.rect(x, y - 2, 5, 1).fill(15253837);
  g.rect(x + 5, y - 3, 1, 2).fill(14275268);
  g.rect(x, y - 4, 5, 1).fill(16777215);
}
const SPEED = 48;
const SIT_OFFSET = 5;
const SIT_OFFSET_DOWN = 12;
const SIT_OFFSET_UP = 5;
const SIT_OFFSET_SIDE = 4;
const SEAT_LEG_CROP = 8;
const SEAT_BACK_CROP = 2;
const IDLE_LINGER_SECONDS = 30;
const DESK_REST_SECONDS = 30;
const NAME_TAG_FONT_SIZE = 6;
const NAME_TAG_STROKE_WIDTH = 1;
const NAME_TAG_OFFSET_Y = -4;
const NAME_TAG_PAD_X = 2;
const NAME_TAG_PAD_Y = 1;
const NAME_TAG_RADIUS = 2;
const NAME_TAG_PLATE_ALPHA = 0.82;
const NAME_TAG_Z = 5e4;
class Character {
  agentId;
  sprite;
  state = "idle";
  mapRenderer;
  deskTile;
  seatDirection;
  px;
  py;
  path = [];
  pendingWork = null;
  pendingSit = false;
  sitting = false;
  wandering = false;
  idleTimer = 0;
  idleWanderDelay = 1 + Math.random() * 3;
  // Idle 30/30 loop state (see constants above). Active only between tasks.
  idleLoop = false;
  idleLoopPhase = "linger";
  idleLoopTimer = 0;
  direction = "down";
  arrivalCallback = null;
  isVisible = false;
  fadeDirection = null;
  fadeDuration = 0;
  fadeElapsed = 0;
  hideTimer = null;
  thoughtBubble;
  /** Persistent nameplate below the feet — unlike the thought bubble, never
   *  fades independently. World-positioned rather than parented to the sprite
   *  so it can sort above every avatar (see NAME_TAG_Z); `plate` is the opaque
   *  backing that keeps it readable over whatever it lands on. */
  nameTag;
  nameTagText;
  nameTagPlate;
  workGlow;
  workGlowElapsed = 0;
  glowOn = false;
  overlay;
  statusGlyph = "none";
  glyphElapsed = 0;
  onClick;
  noNameTag;
  // ── Office-life effects (cheer / coffee / watering) ────────────────────────
  /** Effect layer riding on the sprite: confetti, the carried cup, droplets. */
  fx;
  fxDirty = false;
  // fx drew last frame → needs a clear when idle
  cheerT = -1;
  // -1 = not cheering
  confetti = [];
  carryingCup = false;
  /** The cup parked on this agent's desk (world-positioned, lives in the char
   *  layer so it survives the agent walking away). */
  deskCup;
  deskCupOn = false;
  cupSpot = null;
  waterT = -1;
  // -1 = not watering
  waterDur = 0;
  onWaterDone = null;
  smokeT = -1;
  // -1 = not smoking (the boss's cigar)
  smokeDur = 0;
  onSmokeDone = null;
  constructor(options) {
    this.agentId = options.agentId;
    this.mapRenderer = options.mapRenderer;
    this.sprite = new CharacterSprite(options.frames);
    this.deskTile = options.seatTile;
    this.seatDirection = options.seatDirection ?? "down";
    this.onClick = options.onClick;
    this.noNameTag = options.noNameTag === true;
    const start = options.spawnTile ?? this.deskTile;
    const pos = this.mapRenderer.tileToPixel(start.x, start.y);
    this.px = pos.x + this.mapRenderer.tileSize / 2;
    this.py = pos.y + this.mapRenderer.tileSize;
    this.sprite.setPosition(this.px, this.py);
    this.thoughtBubble = new ThoughtBubble();
    const area = options.bubbleArea;
    if (area) this.thoughtBubble.setBounds(area.w, area.h, area.x, area.y);
    else this.thoughtBubble.setBounds(
      this.mapRenderer.width * this.mapRenderer.tileSize,
      this.mapRenderer.height * this.mapRenderer.tileSize
    );
    this.nameTag = new Container();
    this.nameTag.zIndex = NAME_TAG_Z;
    this.nameTag.eventMode = "none";
    this.nameTagPlate = new Graphics();
    this.nameTagText = new Text({
      text: options.displayName,
      style: {
        fontSize: NAME_TAG_FONT_SIZE,
        fontWeight: "bold",
        fill: colors.cream[50],
        stroke: { color: colors.ink[900], width: NAME_TAG_STROKE_WIDTH, join: "round" },
        fontFamily: "monospace",
        align: "center"
      }
    });
    this.nameTagText.anchor.set(0.5, 0);
    this.nameTag.addChild(this.nameTagPlate);
    this.nameTag.addChild(this.nameTagText);
    this.drawNameTagPlate();
    this.workGlow = new Graphics();
    this.workGlow.circle(0, 0, 14);
    this.workGlow.fill({ color: options.glowColor, alpha: 1 });
    this.workGlow.alpha = 0;
    this.workGlow.eventMode = "none";
    this.overlay = new Graphics();
    this.overlay.eventMode = "none";
    this.fx = new Graphics();
    this.fx.eventMode = "none";
    this.deskCup = new Graphics();
    this.deskCup.eventMode = "none";
    this.deskCup.visible = false;
  }
  getAnimation() {
    return this.state;
  }
  getDeskTile() {
    return this.deskTile;
  }
  /** Repaint this figure from new frames without moving it: the sprite
   *  editor saved the custom avatar it is wearing. */
  setFrames(frames) {
    this.sprite.replaceFrames(frames);
  }
  getPixelPosition() {
    return { x: this.px, y: this.py };
  }
  /** The sprite's box on the canvas, in the page's pixels. */
  getScreenBox() {
    const b = this.sprite.container.getBounds();
    return { x: b.x, y: b.y, w: b.width, h: b.height };
  }
  getTilePosition() {
    return this.mapRenderer.pixelToTile(this.px, this.py - 1);
  }
  moveTo(tile) {
    const path = findPath(this.mapRenderer, this.getTilePosition(), tile);
    if (path && path.length > 0) {
      this.sitting = false;
      this.sprite.setSeatedCrop(0);
      this.path = path;
      this.state = "walk";
      this.sprite.setAnimation("walk", this.direction);
    }
  }
  walkToAndThen(tile, callback) {
    this.idleLoop = false;
    this.arrivalCallback = callback;
    this.moveTo(tile);
    if (this.state !== "walk") {
      this.arrivalCallback = null;
      const t = this.getTilePosition();
      if (t.x === tile.x && t.y === tile.y) callback();
    }
  }
  /** Sit at the assigned desk, facing the monitor. Walks there first if away.
   *  `working` toggles the pulsing focus halo. This is the default pose — agents
   *  stay seated unless blocked. */
  sitAtDesk(working) {
    this.idleLoop = false;
    this.walkToDeskAndSit(working);
  }
  /** Walk to the home desk (if away) and sit. `working` toggles the focus halo.
   *  Shared by sitAtDesk (real work/wait) and the idle-loop rest. */
  walkToDeskAndSit(working) {
    this.glowOn = working;
    this.wandering = false;
    const t = this.getTilePosition();
    if (t.x === this.deskTile.x && t.y === this.deskTile.y) {
      this.applySit();
    } else {
      this.pendingSit = true;
      this.pendingWork = null;
      this.arrivalCallback = null;
      this.moveTo(this.deskTile);
    }
  }
  /** Snap into the seated pose at the current (desk) tile. */
  applySit() {
    this.applySitPose(this.seatDirection);
  }
  /** Snap into a seated pose facing `dir` at the current tile. Shared by the
   *  home desk (applySit) and any café seat (sitInPlace). */
  applySitPose(dir) {
    this.state = "idle";
    this.pendingWork = null;
    this.pendingSit = false;
    this.path = [];
    this.sitting = true;
    this.direction = dir;
    this.sprite.setAnimation("idle", dir);
    let dx = 0, dy = 0;
    switch (dir) {
      case "down":
        dy = SIT_OFFSET_DOWN;
        break;
      case "up":
        dy = SIT_OFFSET_UP;
        break;
      case "left":
        dx = -SIT_OFFSET;
        dy = SIT_OFFSET_SIDE;
        break;
      case "right":
        dx = SIT_OFFSET;
        dy = SIT_OFFSET_SIDE;
        break;
    }
    this.sprite.setPosition(this.px + dx, this.py + dy);
    this.sprite.setSeatedCrop(dir === "down" ? SEAT_LEG_CROP : SEAT_BACK_CROP);
  }
  /** Sit on a café seat at the CURRENT tile, facing `dir`. The agent must have
   *  already walked onto the seat tile (drive this from walkToAndThen). Unlike
   *  sitAtDesk this leaves the agent's home desk untouched and never lights the
   *  focus halo — it's a break, not work. */
  sitInPlace(dir) {
    this.idleLoop = false;
    this.wandering = false;
    this.glowOn = false;
    this.arrivalCallback = null;
    this.applySitPose(dir);
  }
  /** True while the avatar is parked in a seated pose (desk or café). */
  isSitting() {
    return this.sitting;
  }
  /** Turn a standing/idle avatar to face `dir` (e.g. toward the coffee machine
   *  while taking a standing break). No-op mid-walk or while seated. */
  faceDirection(dir) {
    this.direction = dir;
    if (!this.sitting && this.state !== "walk") {
      this.sprite.setAnimation("idle", dir);
    }
  }
  setIdle() {
    this.idleLoop = false;
    this.state = "idle";
    this.pendingWork = null;
    this.pendingSit = false;
    this.sitting = false;
    this.wandering = false;
    this.path = [];
    this.glowOn = false;
    this.sprite.setSeatedCrop(0);
    this.sprite.setAnimation("idle", this.direction);
    this.sprite.setPosition(this.px, this.py);
  }
  /** Roam the office between tasks. Picks random walkable tiles and strolls
   *  to them until the agent is given work again. */
  startWandering() {
    if (this.idleLoop && this.wandering) return;
    this.idleLoop = true;
    this.idleLoopPhase = "linger";
    this.idleLoopTimer = 0;
    this.beginWander();
  }
  /** Low-level: start roaming the floor now. Drives the linger phase of the
   *  idle loop (and is reused when a rest ends). Does not touch the loop state. */
  beginWander() {
    if (this.wandering) return;
    this.glowOn = false;
    this.sitting = false;
    this.pendingSit = false;
    this.pendingWork = null;
    this.wandering = true;
    this.idleTimer = 0;
    this.idleWanderDelay = 0.5 + Math.random() * 2;
    this.sprite.setSeatedCrop(0);
    if (this.state !== "walk") {
      this.state = "idle";
      this.sprite.setAnimation("idle", this.direction);
      this.sprite.setPosition(this.px, this.py);
    }
  }
  /** Walk to an arbitrary tile (e.g. the waiting area when blocked); stands on arrival. */
  walkToTile(tile) {
    this.idleLoop = false;
    this.pendingWork = null;
    this.pendingSit = false;
    this.sitting = false;
    this.wandering = false;
    this.arrivalCallback = null;
    this.moveTo(tile);
  }
  repositionTo(tx, ty) {
    this.deskTile = { x: tx, y: ty };
    const pos = this.mapRenderer.tileToPixel(tx, ty);
    this.px = pos.x + this.mapRenderer.tileSize / 2;
    this.py = pos.y + this.mapRenderer.tileSize;
    this.sprite.setPosition(this.px, this.py);
  }
  /** Show what the agent is doing right now in the thought cloud above its head.
   *  Empty text renders an animated "…" (thinking); `tool` adds a small glyph. */
  showThought(text, tool) {
    this.thoughtBubble.show(text, tool);
  }
  /** Fade the thought cloud out after a short linger — the agent went quiet. */
  hideThought() {
    this.thoughtBubble.startLinger();
  }
  /** The thought cloud's current base screen rect (no lift), or null if hidden.
   *  The scene uses this to detect and resolve overlapping bubbles. */
  getThoughtLayout() {
    return this.thoughtBubble.getLayout(this.px, this.py);
  }
  /** Shift this avatar's thought cloud up by `px` so it clears a nearby one. */
  setThoughtLift(px) {
    this.thoughtBubble.setLift(px);
  }
  /** Forward the camera zoom so the thought cloud can counter-scale and keep
   *  its on-screen text size when the window (and thus the world) shrinks. */
  setBubbleZoom(z) {
    this.thoughtBubble.setZoom(z);
  }
  /** Update the nameplate text (a rename mid-session, e.g. god's name). No-op
   *  when unchanged — cheap to call on every store tick, and the plate is only
   *  repainted when the text it has to cover actually changed. */
  setName(displayName) {
    if (this.nameTagText.text === displayName) return;
    this.nameTagText.text = displayName;
    this.drawNameTagPlate();
  }
  /** Repaint the backing plate to the text's measured width. Sized here rather
   *  than at a fixed width because names run from "QA" to "Architect". */
  drawNameTagPlate() {
    const w = this.nameTagText.width + NAME_TAG_PAD_X * 2;
    const h = this.nameTagText.height + NAME_TAG_PAD_Y * 2;
    this.nameTagPlate.clear().roundRect(-w / 2, -NAME_TAG_PAD_Y, w, h, NAME_TAG_RADIUS).fill({ color: colors.ink[900], alpha: NAME_TAG_PLATE_ALPHA });
  }
  setStatusGlyph(glyph) {
    if (glyph === this.statusGlyph) return;
    this.statusGlyph = glyph;
    this.glyphElapsed = 0;
    if (glyph === "none") this.overlay.clear();
  }
  // ── Cheer ──────────────────────────────────────────────────────────────────
  /** Celebrate finished work: a couple of happy hops under a confetti burst.
   *  Movement (wander / idle loop) is held for the duration so the jump reads
   *  on the spot; whatever the avatar was told to do resumes right after. */
  cheer() {
    if (this.sitting) return;
    this.path = [];
    if (this.state === "walk") {
      this.state = "idle";
      this.sprite.setAnimation("idle", this.direction);
    }
    this.cheerT = 0;
    this.confetti = [];
    const colors2 = [16767293, 16739179, 7064439, 5084927, 16164607];
    for (let i = 0; i < 14; i++) {
      this.confetti.push({
        x: (Math.random() - 0.5) * 8,
        y: -22 - Math.random() * 6,
        vx: (Math.random() - 0.5) * 46,
        vy: -30 - Math.random() * 40,
        c: colors2[i % colors2.length]
      });
    }
  }
  /** True while the cheer animation holds the avatar in place. */
  isCheering() {
    return this.cheerT >= 0;
  }
  // ── Coffee cup ─────────────────────────────────────────────────────────────
  /** Where this agent's cup rests when parked on its desk (world pixels).
   *  Typically beside the monitor — where the old baked-in tileset mug sat. */
  setCupSpot(spot) {
    this.cupSpot = spot;
    if (spot) {
      this.deskCup.position.set(spot.x, spot.y);
      this.deskCup.zIndex = spot.y;
    }
  }
  /** Show/hide the cup in the avatar's hand (walking it to/from the café). */
  setCarryingCup(carrying) {
    this.carryingCup = carrying;
  }
  /** Park the carried cup on the desk / pick it back up. No-op without a spot. */
  setCupOnDesk(on) {
    if (!this.cupSpot) return;
    this.deskCupOn = on;
    this.deskCup.visible = on;
    if (on) this.drawCup(this.deskCup, 0, 0);
    else this.deskCup.clear();
  }
  hasCupOnDesk() {
    return this.deskCupOn;
  }
  isCarryingCup() {
    return this.carryingCup;
  }
  // ── Watering ───────────────────────────────────────────────────────────────
  /** Water the plant the avatar is facing: a held watering can + a steady arc
   *  of droplets for `seconds`, then `onDone` (resume wandering etc.). */
  startWatering(seconds, onDone) {
    this.waterT = 0;
    this.waterDur = seconds;
    this.onWaterDone = onDone ?? null;
  }
  isWatering() {
    return this.waterT >= 0;
  }
  /** Abort a watering in progress (real work arrived). The callback is dropped. */
  stopWatering() {
    this.waterT = -1;
    this.onWaterDone = null;
  }
  // ── The boss's cigar ───────────────────────────────────────────────────────
  /** Light a cigar for `seconds`: a glowing stub in hand and smoke puffs
   *  drifting up. Pure boss energy; pair it with a window for plausible
   *  deniability. `onDone` fires when it's been smoked down. */
  startSmoking(seconds, onDone) {
    this.smokeT = 0;
    this.smokeDur = seconds;
    this.onSmokeDone = onDone ?? null;
  }
  isSmoking() {
    return this.smokeT >= 0;
  }
  /** Stub the cigar out early (real work arrived). The callback is dropped. */
  stopSmoking() {
    this.smokeT = -1;
    this.onSmokeDone = null;
  }
  setBaseAlpha(alpha) {
    this.targetAlpha = alpha;
  }
  targetAlpha = 1;
  enableClick() {
    this.sprite.container.eventMode = "static";
    this.sprite.container.cursor = "pointer";
    this.sprite.container.on("pointertap", (e) => {
      e.stopPropagation();
      this.onClick?.(this.agentId);
    });
  }
  show(parent) {
    if (this.hideTimer) {
      clearTimeout(this.hideTimer);
      this.hideTimer = null;
    }
    this.isVisible = true;
    this.sprite.setAlpha(0);
    parent.addChild(this.workGlow);
    parent.addChild(this.sprite.container);
    this.sprite.container.addChild(this.overlay);
    this.sprite.container.addChild(this.fx);
    if (!this.noNameTag) parent.addChild(this.nameTag);
    parent.addChild(this.deskCup);
    parent.addChild(this.thoughtBubble.container);
    this.enableClick();
    this.fadeDirection = "in";
    this.fadeDuration = 0.5;
    this.fadeElapsed = 0;
  }
  hide(delay = 0) {
    if (this.hideTimer) {
      clearTimeout(this.hideTimer);
      this.hideTimer = null;
    }
    const begin = () => {
      this.hideTimer = null;
      this.fadeDirection = "out";
      this.fadeDuration = 0.6;
      this.fadeElapsed = 0;
    };
    if (delay > 0) this.hideTimer = setTimeout(begin, delay);
    else begin();
  }
  update(dt) {
    if (this.fadeDirection) {
      this.fadeElapsed += dt;
      const t = Math.min(this.fadeElapsed / this.fadeDuration, 1);
      const alpha = (this.fadeDirection === "in" ? t : 1 - t) * this.targetAlpha;
      this.sprite.setAlpha(alpha);
      if (t >= 1) {
        const reachedZero = this.fadeDirection === "out";
        this.fadeDirection = null;
        if (reachedZero) {
          this.isVisible = false;
          this.sprite.container.parent?.removeChild(this.sprite.container);
          this.thoughtBubble.hide();
          this.thoughtBubble.container.parent?.removeChild(this.thoughtBubble.container);
          this.workGlow.parent?.removeChild(this.workGlow);
          this.nameTag.parent?.removeChild(this.nameTag);
          this.deskCup.parent?.removeChild(this.deskCup);
        }
      }
    } else if (this.isVisible) {
      const a = this.sprite.container.alpha;
      if (Math.abs(a - this.targetAlpha) > 0.01) {
        this.sprite.setAlpha(lerp(a, this.targetAlpha, Math.min(1, dt / 0.2)));
      }
    }
    this.thoughtBubble.update(dt);
    if (!this.isVisible) return;
    this.sprite.advance(dt);
    const heldByFx = this.cheerT >= 0 || this.waterT >= 0 || this.smokeT >= 0;
    if (this.state === "walk") this.updateWalk(dt);
    else if (this.wandering && !heldByFx) this.updateWander(dt);
    if (this.idleLoop && !heldByFx) this.updateIdleLoop(dt);
    this.sprite.container.zIndex = this.py;
    this.thoughtBubble.setPosition(this.px, this.py);
    this.nameTag.x = Math.round(this.px);
    this.nameTag.y = Math.round(this.py + NAME_TAG_OFFSET_Y);
    this.nameTag.zIndex = NAME_TAG_Z + this.py;
    this.nameTag.alpha = this.sprite.container.alpha;
    const ts = this.mapRenderer.tileSize;
    this.workGlow.x = this.px;
    this.workGlow.y = this.py - ts / 2;
    this.workGlow.zIndex = this.py - 1;
    if (this.glowOn) {
      this.workGlowElapsed += dt;
      const phase = (Math.sin(this.workGlowElapsed * Math.PI / 0.6) + 1) / 2;
      this.workGlow.alpha = (0.18 + 0.27 * phase) * this.sprite.container.alpha;
      this.workGlow.scale.set(0.95 + 0.15 * phase);
    } else {
      this.workGlow.alpha = 0;
      this.workGlowElapsed = 0;
    }
    this.updateStatusGlyph(dt);
    this.updateFx(dt);
  }
  /** True while parked in the seated pose at the HOME desk (not a café seat). */
  isSittingAtDesk() {
    if (!this.sitting) return false;
    const t = this.getTilePosition();
    return t.x === this.deskTile.x && t.y === this.deskTile.y;
  }
  // ── Effect rendering (cheer confetti, carried cup, watering, cup steam) ────
  /** Carried-cup offset from the feet anchor, per facing direction. */
  carryOffset() {
    switch (this.direction) {
      case "left":
        return { x: -7, y: -9 };
      case "right":
        return { x: 7, y: -9 };
      case "up":
        return { x: -5, y: -10 };
      default:
        return { x: 5, y: -9 };
    }
  }
  /** Hand position while watering, per facing direction. */
  handOffset() {
    switch (this.direction) {
      case "left":
        return { x: -6, y: -9 };
      case "right":
        return { x: 6, y: -9 };
      case "up":
        return { x: 0, y: -13 };
      default:
        return { x: 0, y: -6 };
    }
  }
  drawCup(g, x, y) {
    paintCup(g, x, y);
  }
  steamT = 0;
  updateFx(dt) {
    this.steamT += dt;
    if (this.deskCupOn && this.cupSpot) {
      this.deskCup.clear();
      this.drawCup(this.deskCup, 0, 0);
      for (let i = 0; i < 2; i++) {
        const ph = (this.steamT * 0.7 + i * 0.5) % 1;
        this.deskCup.rect(1 + i * 2, -5 - Math.round(ph * 5), 1, 1).fill({ color: 16777215, alpha: 0.5 * (1 - ph) });
      }
    }
    const active = this.cheerT >= 0 || this.waterT >= 0 || this.smokeT >= 0 || this.carryingCup;
    if (!active) {
      if (this.fxDirty) {
        this.fx.clear();
        this.fxDirty = false;
      }
      return;
    }
    this.fx.clear();
    this.fxDirty = true;
    if (this.cheerT >= 0) {
      this.cheerT += dt;
      const t = this.cheerT;
      if (t >= 1.6) {
        this.cheerT = -1;
        this.sprite.setPosition(this.px, this.py);
      } else {
        const decay = 1 - t / 1.6;
        const hop = Math.abs(Math.sin(t * Math.PI * 2.2)) * 5 * decay;
        this.sprite.setPosition(this.px, this.py - hop);
        for (const p of this.confetti) {
          p.x += p.vx * dt;
          p.y += p.vy * dt;
          p.vy += 110 * dt;
          const alpha = Math.max(0, Math.min(1, (1.45 - t) / 0.5));
          this.fx.rect(Math.round(p.x), Math.round(p.y), 2, 2).fill({ color: p.c, alpha });
        }
      }
    }
    if (this.carryingCup) {
      const o = this.carryOffset();
      this.drawCup(this.fx, o.x, o.y);
      const ph = this.steamT * 0.9 % 1;
      this.fx.rect(o.x + 2, o.y - 5 - Math.round(ph * 4), 1, 1).fill({ color: 16777215, alpha: 0.5 * (1 - ph) });
    }
    if (this.smokeT >= 0) {
      this.smokeT += dt;
      if (this.smokeT >= this.smokeDur) {
        this.smokeT = -1;
        const cb = this.onSmokeDone;
        this.onSmokeDone = null;
        cb?.();
      } else {
        const h = this.handOffset();
        const dirX = this.direction === "left" ? -1 : this.direction === "right" ? 1 : 0;
        const tipX = h.x + (dirX >= 0 ? 4 : -4);
        this.fx.rect(Math.min(h.x, tipX), h.y - 1, 4, 1).fill(7031347);
        this.fx.rect(h.x + (dirX >= 0 ? 1 : -2), h.y - 1, 1, 1).fill(14262346);
        const ember = 0.5 + 0.5 * Math.sin(this.smokeT * 5);
        this.fx.rect(tipX, h.y - 1, 1, 1).fill({ color: 16742972, alpha: 0.55 + 0.45 * ember });
        for (let i = 0; i < 3; i++) {
          const ph = (this.smokeT * 0.45 + i / 3) % 1;
          const px2 = tipX + Math.sin((this.smokeT + i * 2) * 1.7) * 2 + ph * 2 * (dirX || 1);
          const py2 = h.y - 3 - ph * 12;
          this.fx.circle(px2, py2, 1 + ph * 1.5).fill({ color: 13617876, alpha: 0.45 * (1 - ph) });
        }
      }
    }
    if (this.waterT >= 0) {
      this.waterT += dt;
      if (this.waterT >= this.waterDur) {
        this.waterT = -1;
        const cb = this.onWaterDone;
        this.onWaterDone = null;
        cb?.();
      } else {
        const h = this.handOffset();
        const dirX = this.direction === "left" ? -1 : this.direction === "right" ? 1 : 0;
        const dirY = this.direction === "up" ? -1 : this.direction === "down" ? 1 : 0;
        this.fx.rect(h.x - 2, h.y - 2, 5, 3).fill(10135472);
        this.fx.rect(h.x + (dirX >= 0 ? 3 : -4), h.y - 2, 2, 1).fill(10135472);
        for (let i = 0; i < 4; i++) {
          const ph = (this.waterT * 1.3 + i / 4) % 1;
          const reach = 4 + ph * 7;
          const dx = dirX !== 0 ? reach * dirX : (i - 1.5) * 1.5;
          const dy = dirY !== 0 ? reach * dirY : 0;
          const fall = ph * ph * 9;
          this.fx.rect(Math.round(h.x + dx), Math.round(h.y + dy + fall - 2), 1, 2).fill({ color: 6010856, alpha: 1 - ph * 0.45 });
        }
      }
    }
  }
  updateStatusGlyph(dt) {
    if (this.statusGlyph === "none") return;
    this.glyphElapsed += dt;
    const g = this.overlay;
    g.clear();
    const yTop = -34;
    if (this.statusGlyph === "blocked") {
      if (Math.floor(this.glyphElapsed / 0.4) % 2 === 0) {
        g.rect(-1, yTop, 2, 5).fill(16739179);
        g.rect(-1, yTop + 6, 2, 2).fill(16739179);
      }
    } else if (this.statusGlyph === "success") {
      const p = (Math.sin(this.glyphElapsed * 18) + 1) / 2;
      const s = 2 + p * 2;
      g.rect(-0.5, yTop - s, 1, s * 2).fill(16767293);
      g.rect(-s, yTop - 0.5, s * 2, 1).fill(16767293);
      if (this.glyphElapsed > 0.9) this.setStatusGlyph("none");
    } else if (this.statusGlyph === "compacting") {
      const p = (Math.sin(this.glyphElapsed * 6) + 1) / 2;
      const s = 2 + p * 3;
      g.rect(-s, yTop - s, s * 2, s * 2).fill(10190558);
    } else if (this.statusGlyph === "looping") {
      const idx = Math.floor(this.glyphElapsed * 8) % 4;
      const pts = [[-3, yTop - 3], [3, yTop - 3], [3, yTop + 3], [-3, yTop + 3]];
      for (let i = 0; i < 4; i++) {
        const [x, y] = pts[i];
        g.rect(x - 1, y - 1, 2, 2).fill(i === idx ? 16752451 : 7035e3);
      }
    }
  }
  updateWalk(dt) {
    if (this.path.length === 0) {
      if (this.pendingSit) {
        this.applySit();
      } else if (this.pendingWork) {
        this.state = this.pendingWork;
        this.pendingWork = null;
        this.sprite.setAnimation(this.state, this.seatDirection);
      } else if (this.wandering) {
        this.state = "idle";
        this.idleTimer = 0;
        this.idleWanderDelay = 1 + Math.random() * 3;
        this.sprite.setAnimation("idle", this.direction);
      } else {
        this.setIdle();
      }
      if (this.arrivalCallback) {
        const cb = this.arrivalCallback;
        this.arrivalCallback = null;
        cb();
      }
      return;
    }
    const ts = this.mapRenderer.tileSize;
    const fromX = this.px;
    const fromY = this.py;
    const step = advanceAlongPath(
      this.px,
      this.py,
      this.path,
      (tile) => ({ x: tile.x * ts + ts / 2, y: tile.y * ts + ts }),
      SPEED * dt
    );
    this.px = step.x;
    this.py = step.y;
    if (step.consumed > 0) this.path.splice(0, step.consumed);
    const dx = this.px - fromX;
    const dy = this.py - fromY;
    if (dx !== 0 || dy !== 0) {
      this.direction = Math.abs(dx) > Math.abs(dy) ? dx > 0 ? "right" : "left" : dy > 0 ? "down" : "up";
    }
    this.sprite.setAnimation("walk", this.direction);
    this.sprite.setPosition(this.px, this.py);
  }
  /** Drive the idle 30/30 loop: linger on the floor, then rest at the desk,
   *  then linger again — independent of the low-level walk/wander animation. */
  updateIdleLoop(dt) {
    switch (this.idleLoopPhase) {
      case "linger":
        this.idleLoopTimer += dt;
        if (this.idleLoopTimer >= IDLE_LINGER_SECONDS) {
          this.idleLoopPhase = "toDesk";
          this.idleLoopTimer = 0;
          this.walkToDeskAndSit(false);
        }
        break;
      case "toDesk":
        this.idleLoopTimer += dt;
        if (this.sitting) {
          this.idleLoopPhase = "resting";
          this.idleLoopTimer = 0;
        } else if (this.idleLoopTimer >= 20) {
          this.idleLoopPhase = "linger";
          this.idleLoopTimer = 0;
          this.beginWander();
        }
        break;
      case "resting":
        this.idleLoopTimer += dt;
        if (this.idleLoopTimer >= DESK_REST_SECONDS) {
          this.idleLoopPhase = "linger";
          this.idleLoopTimer = 0;
          this.beginWander();
        }
        break;
    }
  }
  updateWander(dt) {
    this.idleTimer += dt;
    if (this.idleTimer < this.idleWanderDelay) return;
    this.idleTimer = 0;
    this.idleWanderDelay = 1 + Math.random() * 3;
    const cur = this.getTilePosition();
    const range = 6;
    for (let attempt = 0; attempt < 14; attempt++) {
      const tx = cur.x + Math.floor(Math.random() * range * 2) - range;
      const ty = cur.y + Math.floor(Math.random() * range * 2) - range;
      if ((tx !== cur.x || ty !== cur.y) && this.mapRenderer.isWalkable(tx, ty)) {
        const wasWandering = this.wandering;
        this.moveTo({ x: tx, y: ty });
        this.wandering = wasWandering;
        return;
      }
    }
  }
  destroy() {
    if (this.hideTimer) {
      clearTimeout(this.hideTimer);
      this.hideTimer = null;
    }
    this.thoughtBubble.destroy();
    this.sprite.destroy();
    this.workGlow.destroy();
    this.overlay.destroy();
    this.fx.destroy();
    this.nameTag.parent?.removeChild(this.nameTag);
    this.nameTag.destroy({ children: true });
    this.deskCup.parent?.removeChild(this.deskCup);
    this.deskCup.destroy();
  }
}
const DEFAULT_ON_GIDS = [
  // [gid, dx, dy] in tiles relative to the block's top-left
  [367, 0, 0],
  [368, 1, 0],
  [383, 0, 1],
  [384, 1, 1]
];
const SCREEN = { x: 3, y: 5, h: 12 };
class DeskScreen {
  container = new Container();
  anim = new Graphics();
  on = false;
  t = 0;
  constructor(mapRenderer, topLeft, monitor) {
    const ts = mapRenderer.tileSize;
    const onGids = monitor?.onGids ?? DEFAULT_ON_GIDS;
    for (const [gid, dx, dy] of onGids) {
      const tex = mapRenderer.textureForGid(gid);
      if (!tex) continue;
      const s = new Sprite(tex);
      s.x = dx * ts;
      s.y = dy * ts;
      this.container.addChild(s);
    }
    this.anim.eventMode = "none";
    this.container.addChild(this.anim);
    this.container.x = topLeft.x * ts;
    this.container.y = topLeft.y * ts;
    this.container.zIndex = (topLeft.y + 2) * ts - 1;
    this.container.visible = false;
    this.container.eventMode = "none";
  }
  /** Light the screen (agent sat down) or cut it (stood up / left). */
  setOn(on) {
    if (on === this.on) return;
    this.on = on;
    this.container.visible = on;
    if (!on) {
      this.anim.clear();
      this.t = 0;
    }
  }
  update(dt) {
    if (!this.on) return;
    this.t += dt;
    const g = this.anim;
    g.clear();
    for (let i = 0; i < 2; i++) {
      const phase = (this.t * 3.2 + i * (SCREEN.h / 2)) % SCREEN.h;
      const y = SCREEN.y + SCREEN.h - 1 - phase;
      const w = 6 + (i * 7 + Math.floor(this.t / 1.7)) % 9;
      g.rect(SCREEN.x + 2, Math.round(y), w, 1).fill({ color: 13625087, alpha: 0.55 });
    }
    if (Math.floor(this.t / 0.53) % 2 === 0) {
      g.rect(SCREEN.x + 2, SCREEN.y + SCREEN.h - 2, 2, 2).fill({ color: 16777215, alpha: 0.9 });
    }
  }
  destroy() {
    this.container.destroy({ children: true });
  }
}
const frameCache = /* @__PURE__ */ new Map();
function bufToTexture(buf) {
  const canvas = document.createElement("canvas");
  canvas.width = SCENE_W;
  canvas.height = SCENE_H;
  const ctx = canvas.getContext("2d");
  const img = ctx.createImageData(SCENE_W, SCENE_H);
  img.data.set(buf);
  ctx.putImageData(img, 0, 0);
  const tex = Texture.from(canvas);
  tex.source.scaleMode = "nearest";
  return tex;
}
async function getCastFrames(name) {
  const recipe = resolveRecipe(name);
  const key = `${name}|${recipeKey(recipe)}|${costumeOf(recipe) ?? ""}`;
  const cached = frameCache.get(key);
  if (cached) return cached;
  const { front, back } = sceneFrameBufs(name);
  const toRow = (bufs) => {
    const [stand, stepL, stepR] = bufs.map(bufToTexture);
    return [stand, stepL, stepR, stand, stand, stand, stand];
  };
  const frontRow = toRow(front);
  const frames = [frontRow, toRow(back), frontRow];
  frameCache.set(key, frames);
  return frames;
}
const pick = (arr, seed) => arr[(seed % arr.length + arr.length) % arr.length];
const COFFEE = [
  "is this… decaf?? who did this",
  "we're out of beans again",
  "World’s Best Boss mug",
  "first cup of the day. and the fifth.",
  "the coffee here is basically a hug",
  "who took my mug?"
];
const VENDING = [
  "the machine ate my dollar",
  "B4… please be the pretzels",
  "it’s stuck. classic.",
  "shaking it. gently. respectfully.",
  "one (1) emotional-support snack",
  "A1 again. living dangerously."
];
const SNACK = [
  "is it Pretzel Day?",
  "who finished the chips??",
  "just a little treat",
  "these are everyone’s? cool cool cool",
  "second breakfast"
];
const TABLE = [
  "big day. lots of meetings.",
  "just five more minutes",
  "did you see the standup notes?",
  "pretending to read my notes",
  "I needed this break, honestly",
  "do NOT tell Michael I’m in here"
];
const SPOT_POOL = {
  coffee: COFFEE,
  vending: VENDING,
  snack: SNACK,
  table: TABLE
};
const BY_CHARACTER = {
  michael: ["I DECLARE… BANKRUPTCY!", "that's what she said", "I'm not superstitious. just a little stitious.", "no meetings before coffee. that’s the rule."],
  dwight: ["FALSE.", "identity theft is not a joke", "that mug is regulation", "this fridge needs a beet drawer", "Schrute Farms has better coffee"],
  jim: ["...that's what she said", "bears. beets. Battlestar Galactica.", "I moved Dwight’s stapler again", "just here for the gossip"],
  pam: ["Dunder Mifflin, this is Pam", "sketching the vending machine", "the watercolor of the break room"],
  kevin: ["the chili is NOT ready", "why waste time say lot word", "me want snack", "cookie? cookie."],
  angela: ["this break room is filthy", "party planning committee, 3pm", "I’m judging the fridge"],
  oscar: ["actually, it’s “espresso”", "well, actually…", "the budget for snacks is concerning"],
  stanley: ["is it Pretzel Day?", "did I stutter?", "crossword and coffee. leave me be.", "I'll retire before this brews"],
  phyllis: ["Bob is picking me up at five", "knitting and a nice cup of tea"],
  andy: ["Cornell, ever heard of it?", "rit-dit-dit, coffee break!", "Big Tuna, grab a chair"],
  kelly: ["did you HEAR what happened??", "so. much. to tell you.", "I am the GOSSIP queen"],
  ryan: ["I’m kind of a big deal", "the temp needs caffeine", "starting a coffee startup, actually"],
  toby: ["I should write that up…", "HR-wise this break is fine", "no one ever sits with me"],
  creed: ["which one of you is the new guy?", "I’ve eaten worse out of that fridge", "mung beans. under my desk."],
  meredith: ["is it 5 o’clock yet?", "someone spike the coffee?"]
};
function pickSoloLine(character, spot, seed) {
  const flavour = BY_CHARACTER[character];
  if (flavour && seed % 5 < 3) return pick(flavour, Math.floor(seed / 5));
  return pick(SPOT_POOL[spot], seed);
}
const EXCHANGES = [
  ["world’s best boss.", "you are. I had the mug made.", "and I cherish it."],
  ["would an idiot do this?", "...if yes, I don’t.", "that’s my boy."],
  ["feared or loved? both.", "that’s beautiful.", "I know."],
  ["I edited your wiki page again.", "I know. thank you."],
  ["question. how many bears?", "one.", "that’s too many."],
  ["fact: bears eat beets.", "bears. beets. Galactica.", "what is happening."],
  ["I grew up on a beet farm.", "shocking.", "...not shocking at all."],
  ["what’s Schrute Farms smell like?", "victory. and beets."],
  ["did you just throw your phone?", "didn’t like what it said.", "cool."],
  ["is a hot dog a sandwich?", "it is.", "I know, right?"],
  ["three-hole-punch Jim returns.", "never gets old."],
  ["why few word when lot word?", "...genuinely profound.", "I know."],
  ["I am not a bad person.", "...", "not a great person either.", "there it is."],
  ["I love my cats more than people.", "including us?", "especially you."],
  ["cats are better than dogs.", "dogs are better.", "...sorry."],
  ["do you love me?", "I love… being here.", "that’s a yes."],
  ["I’m kind of a big deal.", "you are?", "in my mind. yes."],
  ["did you miss me?", "no.", "a little?", "...there it is."],
  ["did you just roll your eyes?", "I did.", "why?", "muscle memory."],
  ["I’ve watched that clock since 4.", "weren’t you working?", "watching the clock."],
  ["what do we sell again?", "paper.", "sure, yeah."],
  ["how old are you?", "yeah.", "that’s not an answer.", "sure it is."],
  ["that’s not how math works.", "I know.", "then why?", "faster."],
  ["I’m not an alcoholic.", "you went to a meeting.", "for the food."],
  ["I went to Cornell.", "nobody cares.", "I went to Cornell.", "still nobody cares."],
  ["I have a lot of feelings.", "I can tell.", "is that bad?", "for us? yes."],
  ["why are you the way you are?", "...", "honestly."],
  ["your cat died.", "I know.", "I’m sorry.", "...thank you."],
  ["stop looking at me.", "you stop looking at me."],
  ["sign this.", "what is it?", "doesn’t matter.", "...fine."],
  ["you can’t say that.", "I just did.", "gonna stop me?", "...no."],
  ["that’s a fire lane.", "fire hasn’t happened yet."],
  ["I wrapped your stapler in Jello.", "I’ll eat around it.", "fair."],
  ["zombie attack plan?", "especially that.", "of course."],
  ["just seeing if you’d answer.", "I hate you.", "I know."],
  ["a little stitious, not super.", "that’s not a word.", "it is now."],
  ["funniest person in the office?", "and other times?", "other times I know it."],
  ["that’s what she said.", "...every time.", "come on."],
  ["I started the fire.", "no you didn’t.", "in our hearts, I did."],
  ["is today a day ending in Y?", "yes.", "then no."],
  ["Bob Vance.", "Phyllis Vance.", "Vance Refrigeration."],
  ["you look beautiful today.", "...I know."],
  ["I’m better than you in every way.", "probably.", "definitely.", "sure."],
  ["I’m a nice guy.", "you’re okay.", "nicest thing you’ve said."],
  ["are you okay?", "I’ve been worse.", "when?", "can’t narrow it down."],
  ["there’s a spider on your desk.", "where?", "...you ate it.", "protein."],
  ["soul mates can be bosses.", "you’re my boss.", "exactly."],
  ["standup ran 40 minutes.", "could’ve been an email."],
  ["is the build green yet?", "...don’t look."],
  ["who reply-all’d everyone?", "we don’t talk about it."]
];
const TWSS_EXCHANGES = [
  ["taking way longer than I expected.", "that’s what she said."],
  ["it’s too big, can’t fit it in my mouth.", "that’s what she said."],
  ["you really need to slow down.", "that’s what she said."],
  ["gonna need a bigger one.", "that’s what she said."],
  ["help, I can’t get it to go in.", "that’s what she said."],
  ["it’s not that hard if you just push.", "that’s what she said."],
  ["I can’t do this all night.", "that’s what she said."],
  ["I need it now, I can’t wait.", "that’s what she said."],
  ["so hot in here, I’m sweating.", "that’s what she said."],
  ["it keeps slipping out of my hands.", "that’s what she said."],
  ["why not just stick it in already?", "that’s what she said.", "*looks at camera*"],
  ["I just need a few more inches.", "that’s what she said.", "for the shelf!", "still counts."],
  ["make it louder, I can barely feel it.", "that’s what she said."],
  ["can we get this over with quickly?", "that’s what she said.", "I meant the meeting.", "sure."],
  ["I just need you to hold it steady.", "that’s what she said."],
  ["can’t believe I did that all morning.", "that’s what she said."],
  ["my hands are cramping.", "that’s what she said.", "from typing!", "that’s what she said."],
  ["hours in and barely halfway done.", "that’s what she said."],
  ["surprisingly heavy for its size.", "that’s what she said."],
  ["be more precise. less sloppy.", "that’s what she said.", "I meant the spreadsheet.", "I know."],
  ["how long was it?", "that’s what she said.", "*the whole room goes quiet*", "I’m sorry, I can’t help it."],
  ["too tight, cutting off my circulation.", "that’s what she said.", "*mouths thank you*"],
  ["I don’t think it’ll fit.", "that’s what she said.", "*stands up and applauds*"],
  ["stop, you’re doing it wrong.", "that’s what she said.", "never been prouder."],
  ["this just keeps getting harder.", "that’s what she said.", "he’s ready."],
  ["not wide enough, I need more room.", "that’s what she said."],
  ["I can hold it a really long time.", "that’s what she said.", "my breath!", "still."],
  ["why is it taking so long?", "that’s what she said.", "I hate you.", "then why set me up?"],
  ["I can’t do it with people watching.", "that’s what she said.", "the presentation!", "sure."],
  ["it’s deeper than it looks.", "that’s what she said.", "the pothole, Michael!", "doesn’t matter."],
  ["so much longer than last time.", "that’s what she said.", "the report, Michael.", "right, right."],
  ["oh my god, it went on FOREVER.", "that’s what she said.", "the Twilight movie!", "classic."],
  ["can’t believe how thick this is.", "that’s what she said.", "the folder. *stares*"],
  ["I fit all THAT in one day?", "that’s what she said.", "that’s actually what I said!", "meta."],
  ["I went at it hard this morning.", "that’s what she said.", "at the gym!", "irrelevant."],
  ["someone help me finish this off.", "that’s what she said.", "the leftover cake!", "still works."],
  ["get in, do my thing, get out.", "that’s what she said.", "*doesn’t look up from crossword*"],
  ["can’t believe it took this long.", "that’s what she said.", "the raise. eight years.", "that one’s on me."],
  ["do it slower, it’ll hurt less.", "that’s what she said.", "for the quarterly review.", "sure, Oscar."],
  ["didn’t realize how big it’d be.", "that’s what she said.", "the calzone, it’s enormous!", "I love this office."],
  ["*to no one* that’s what she said.", "nobody said anything.", "just thinking about earlier."],
  ["*on the phone* that’s what she said.", "who was that?", "my mother. about a sandwich."],
  ["too hot in here! that’s what she said.", "you said both parts.", "I contain multitudes."],
  ["*at the TV* that’s what she said.", "you’re alone, Michael.", "she doesn’t know that."],
  ["you need to be more professional.", "that’s what she said.", "I am she.", "...that’s what she said."],
  ["stop. just stop. every time…", "that’s what she said.", "*leaves the room*", "*whispers* that’s what she said."],
  ["as you can see, it’s going up.", "that’s what she said.", "*everyone groans*", "set that one up myself."],
  ["I declared bankruptcy once. felt good.", "what does that have to do with…", "that’s what she said.", "it doesn’t.", "I know."],
  ["you didn’t say it.", "I know.", "why not?", "I’m growing.", "...that’s what she said.", "there it is."],
  ["impressive you held back today.", "thank you.", "I counted zero times.", "that’s what she said.", "still counts."]
];
const PAIR_POOL = [...EXCHANGES, ...TWSS_EXCHANGES];
const KEYED_EXCHANGES = {
  michael: ["that’s what she said.", "...there it is."],
  dwight: ["identity theft is not a joke.", "nobody touched your stapler, Dwight."],
  kevin: ["why few word when lot word?", "...just use the words, Kevin."],
  kelly: ["okay don’t freak out, but…", "I’m already freaking out."],
  oscar: ["well, actually…", "...here we go."],
  angela: ["this table is filthy.", "it’s a break room, Angela."],
  creed: ["which one are you again?", "...we sit next to each other."],
  stanley: ["is it Pretzel Day?", "no, Stanley.", "...did I stutter?"],
  andy: ["I went to Cornell.", "nobody cares.", "...I went to Cornell."],
  jim: ["question.", "yes.", "nothing. just checking."]
};
function pickExchange(speaker, seed) {
  const keyed = KEYED_EXCHANGES[speaker];
  if (keyed && seed % 4 === 0) return keyed;
  return pick(PAIR_POOL, seed);
}
const officeTilesetUrl = "" + new URL("office-tileset-CHQT-uYL.png", import.meta.url).href;
const a5FloorsWallsUrl = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAQAAAAIACAYAAABtmrL7AAAAAXNSR0IArs4c6QAADmFJREFUeJzt3c2PHGedB/Bf2x2P7RBAWUI2K0GQ4oNfolXyB5BEEV57j7snJEBIcIhIQFGQwj/BASGUSDmtEBxX4oitQVbs3Ts+rF+EyMGJFhSSzSpvtscZezi0n4mnPD3V1V0vPfP7fC6VaXfVU1Vd+T6/rup6ahQteer40YiIOHnqVERE/PwXvyyvb7TVxhAuXr46ioh49ZWXIyJi9ezZ8vqwK1ZR9n9E7Or9vQuMYgk//3ntG3oFgOGM215gtef/46UrbTfRq6dPHNuIyXaNYmtPu9Rm3e9PnzjW6P17RdPtLu/fa1QAkFjrFcBe6fmLsh2lEijfAWEvUAFAYgIAEhMAkJgAgMQEACTW+lWAWS3bddW9ctWiaLp/l+3z6EvW7S5UAJDYYBVAsXruzKDtn3z+9KDtd2XW/Vq2f+jPoW9Nt3uvHicqAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhs1NaCnjp+tPznRkTEHy9d2fH9T5841lbTrWiwvqOIiIuXry7U3j37q20bXS2YiDb/n7nXosfTvFQAkNh46BXI6oUXX4qIiDdef02Pvbu0+nm98OJLo4iIH/34J20udmYqAEhssAqg7jv3XvfEkSMRA+6H6jmYbJ/Hsmz/6tkzg7RbqAAgMecABnLy1OmhV4ElMPRxoAKAxAQAJCYAILHWzwFcvHx1FJOzrDP9InDZlbPFZbva+gVfOfs79HdAhuUqADCY1iuAV195OSIifv6LX26pBHar0vOX7Vo9e7aV5b715z9HRMTPftrNPRFNK69luzdj2XS1f8ovQoeiAoDEurgbMIVlvxuw6d2NiS3F/nI3INC71s4BDJVgu1Xb+2vRiiLb52d/TagAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxFobD6DvEYGWeESeXcn+aKbt/WVEIKB345JkLSZQ16MAtzImWxmN9Y3XX9vVoxZP02AU2z25/U0NuL8GHZNRBQCJdfZ04AajrM71/kU9ceTITO3udWW/Zt8PfVuW5zCoACCxziqAZeeZfKACgNQEACQmACCxtAGwevbM4M9mh6GlDQAg8VWAtZvXI5boeuzQ7IecVACQWOIK4NOIiFg91+55gJPPT35f0NUvIYda37Y13f693v5QVACQ2HivPOe8qWefe2boVYDBqQAgMQEAiQkASEwAQGICABITAJCYAIDEBAAkJgAgMQEAiQkASEwAQGICABITAJCYAIDEBAAkJgAgsfFTx49GRETbIwO9/+61Tt8PbVqWMfr6pgKAxNKOCQj3MiowkI4AgMQEACQmACAxAQCJCQBITABAYmmfDty1ptd5l+W68FCyb/9QVACQmAqgI6vnzuz47yefP73Q+/eauu1v217fn7NSAUBiAgASEwCQmACAxAQAJCYAIDEBAIkJAEhMAEBiAgASEwCQmACAxAQAJCYAIDEBAImlDYDzb16I829eGHo1YFBpAwDocESgpiOuDDVCy24bGWa3rS/LTQUAiXVRAYw6WGZn2h6LrvTQX3n08Ubzzfr+ttf37Jk/jCIiVs9OlnvyVLcVRmnn2PETERFx5fKljU4brHHs+IlBtv+FF1+KiIg3Xn+t0/bqqAAgsdYqgIuXr7a1qF48+9wzQ6/CUjh1+lsREfHIP36j13afOn601/amuXL5UkRE/Mvpfx16VQahAoDEBAAkJgAgMQEAiQkASEwAQGICABITAJCYAIDEBAAkJgAgMQEAiQkASEwAQGICABITAJCYAIDEBAAkJgAgMQEAiQkASEwAQGICABITAJCYAIDEOns68LI7/+aFCE8I2twP5KQCgMTSVgDXrr0dcc/TfHeLttf3e9//bqvLm9du+xz2ChUAJJa2AvjNr39b/nPU8qK7ft59q+t7z34YStv7nwZUAJBY2grg4uWrEfc8p778Pa++n3e/6PoObbev/16hAoDEBAAkJgAgMQEAiQkASEwAQGICABJL+zuArj194lin74c2qAAgMRVAR1bPndnx36t3vzV9P7RBBQCJCQBITABAYgIAEhMAkJgAgMQEACQmACAxAQCJCQBITABAYgIAEhMAkJgAgMQEACQmACAxAQCJCQBITABAYp7N3pLydODvff+7ERHx+ONfj4iIZ597Zsv7zr95ISIiLpz/ry2vP/PsN3d8/7Vrb0dExG9+/dsIT9elJSoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJjTqDzVtm9PHDsRERFHjj25ZdqX26O1yvRWr+1/evOBiIi4vlamB3pt/6sHJ9v96KEy7Xf7r98+GBERN+5sndZZX1/f9vXxeNxo/ocOTqZfPDSZPvyFmWaPz25t3/4DB2Zrv8z/lw8n+/v/125HRMQnG5MHdR9emW05TV1fW787nbR34+7U04EhsXFEbAzR8FtXLkVExOl///YQzcf+jZWIiLi17+NB2n/w4GcREfH+Rw8O0v7fbk62/58fHmb7D++/GRERH6x/udF8paf/5NPrERHxhQcPzzX/e3d74G880mj2zZ7+4w8+iYiIh2YtHSrzP3x3+y9+ODkOvvZIs+1oqlQW//fR1kpPBQCJdfOFA5hJ1z3/tPbeeW9SQakAIDEVACRUKgEVACSmAoCEnAMABABkJgAgMQEAiQkASEwAQGICABLzOwCWQvXuvrq/q/Mtqnp3X93f1fl2KxUAJKYCYClUe/a6v+ter1OtHKo9e93fda/XWZbKQQUAiQkASEwAQGICABITAJCYAIDEBAAk5ncAkJAxAQEVAGRkTEBABQBDKj1xX08IKu0VKgBITAXAoNq6n3/e8QTauitv3vEEimolUO2p51W3PBUAJDb673P/sRE93p9cTcDf/c/BiIg48OhKL+3fendty99H/2Hr8+ZntejINGX+P33wdsQc3wGriT7v/Hc2/ilijv1f3Y/zzv9vT96MmOP4W3RknjJ/Of6q2j4eq/ur2Df6y7avt31OQAUA3Gfc98gk1e9CffX8RWmvJPK8PXd1vmmvT1PeN2/PXZ2v6dnk8r6/jubruauf27TXpynv+/iD9xu1X/1OXff6NJ+/b2sF0NXxWD3upunqasC0cwEqAEhssKsAnyfwlwZpfzPpP7w10/vrevhpZ53bUtfDVxO+7Z6kroev9nBt96R1Pfy0s+6zWrSimaa6nGmVwKIV3TTV5VSPExUAJDZYBbB5Fve9YdrfTODtTwJvmve7fVuVwLzf7dvqQeb9bt9WDzrvd/tZ5+uq568uf1ol8NhGNz1/Me14cDcg4JeA0yzagy9aCSzaEyxaCSzaEy5aCcz7Xb6Y95xA11cB6nR9FaBKBQCJqQAq2j6L37QS6Os74DR9fQeeZtGev2rRqwN7nQoAElMB3NXV9fuirhLo+r7wukqgq+v3RV0l0HUPrRLYngoAEktfAXTd81dVK4E/vTf5LXxfI8JUK4E7G932/FXVSuChJ/vtke+7L3+g36EsCxUAJCYAIDEBAImlPwfQ9V18VdV2ur6Lr6raThkPoOurAEW1nTIeQF9n5+9vZ5i7UZeFCgASS18BFF1XAnXL7boSqFtu1/fz1y236+v0rv9vTwUAiakAKtquBJoup+1KoOly2q4Emi6n7UpAz78zFQAkZkzAKWMCLloJLFpBLFoJLFpBLFoJLFpBLFoJzDvfUOdAir7PAakAIDFjAtaMCdi0Emj7KkLTSqDtHqRpJdB2D9q0Emja89eN2tvVOZDy+jujnUft7eockFGBgRj3fZa0+ky3vn6BVm2vqbpKoOtfEtb1DF3/krCuZ+z6c6yrBBY9jqdVAouatp+qpo3au6hpx0mhAoDENs8BzPoc86ZmXV41geftqavqlrfouP/V12c173gA057xNv/TgRcb97/6+qzmHQ9g2vE099OFKw/n7foqQJ2urwJUqQAgsdHv//NXG9v9Q1sVQDEtof+6dngUEXHlk+3bu7N2Z6729q1sn21leQ8dGEVExKH9k78P75tM12/f3nG54/37J9Px/i2vr6/fbjT/jTuTHmH0wOT9t0eTvw+v7Hxh5vra+t3p1nYOr+xvNv/NA5P1uHs34L6VByfTg/t3mDvizs3bd6dbP5d9B/c1mv+xlc8iIuIroxsREfHoocnvMdZvre84//jAeMu0KPPNOv+7Nybb/7cbk+3/348m612Om1m3p051f31+PE+2e1Smo5sREXFo83Oc7fOsUz1eblSOGxUAJHZfvLTd81eXW60EHluZfDf5wXd+2Em7wHQqAEjsvqsARVtXA6rLqVYC1e9yQH9UAJDYuKuev5j2+4Iy/azmrC3QHRUAJNb7VYCqB5wDgMGoACAxAQCJCQBITABAYgIAEhMAkJgAgMQEACQmACAxAQCJCQBITABAYgIAEhMAkJgAgMTuuxm/7RGB6pZrRCAYjgoAEhvXjdrb9qjA1deNCgzDUQFAYpvd77RKYFF1T3Gte5Yb0B0VACTW+1UAYHmoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAGAX+DsVnwvKX4RkOQAAAABJRU5ErkJggg==";
const interiorsUrl = "" + new URL("interiors-CQGBjffT.png", import.meta.url).href;
const officeMapRaw = '{"compressionlevel":-1,"height":22,"infinite":false,"layers":[{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,1698,1699,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,1714,1715,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"height":22,"id":1,"name":"floor","opacity":1,"type":"tilelayer","visible":true,"width":34,"x":0,"y":0},{"data":[514,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,522,517,530,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,554,533,530,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,533,530,0,0,0,0,0,0,611,611,0,0,0,0,0,0,0,0,0,611,0,0,0,0,0,0,611,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,643,643,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,643,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,643,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,643,643,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,643,643,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,643,0,0,0,0,0,0,0,533,530,522,522,522,522,0,522,522,522,522,522,522,522,0,522,522,522,522,522,0,0,0,0,0,0,643,0,0,0,0,0,0,0,533,530,554,554,554,554,0,554,554,554,554,554,554,554,0,554,554,554,554,554,0,0,0,0,0,0,522,0,522,522,522,522,522,522,533,530,570,570,570,570,0,570,570,570,570,570,570,570,0,570,570,570,570,570,0,0,0,0,0,0,554,0,554,554,554,554,554,554,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,570,0,570,570,570,570,570,570,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,611,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,530,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,643,0,0,0,0,0,0,0,0,533,578,579,579,579,579,579,579,579,579,579,579,579,579,579,579,579,0,579,579,579,579,579,579,579,579,579,579,579,579,579,579,579,579,581],"height":22,"id":2,"name":"walls","opacity":1,"type":"tilelayer","visible":true,"width":34,"x":0,"y":0},{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,266,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,281,282,0,0,0,0,0,0,0,468,0,0,0,0,257,257,257,257,0,0,468,0,0,0,0,0,0,0,0,0,0,0,0,0,297,298,0,0,0,0,289,0,0,0,0,0,0,1610612898,1610612914,2684354789,2684354789,1610612914,1610612930,260,0,0,0,0,0,0,0,0,0,2,3,4,2,3,4,0,0,0,0,2,3,4,0,0,0,0,0,1610612897,1610612913,1610612965,1610612965,1610612913,1610612929,276,0,0,0,0,0,0,0,0,0,0,289,0,0,289,0,0,0,0,0,18,289,20,0,0,0,0,0,0,274,274,274,274,0,0,0,0,2,3,4,2,3,4,0,0,305,0,0,305,0,0,0,0,0,0,305,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,289,0,0,289,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,305,0,0,305,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1029,1026,1027,0,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,0,0,0,0,0,1041,1042,1043,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,0,0,0,0,1044,0,1061,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,0,305,305,0,1073,1074,1075,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,39,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,305,305,0,0,0,297,0,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,2,3,4,0,0,0,0,0,269,191,192,0,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,289,0,0,0,38,40,39,285,207,208,2248,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,305,0,0,0,54,56,55,301,223,224,2264,0,0,470,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,297,0,0,0,0,0,471,0,0,0,0,0,0,0,472,2280,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"height":22,"id":3,"name":"furniture-below","opacity":1,"type":"tilelayer","visible":true,"width":34,"x":0,"y":0},{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,354,327,328,0,0,0,0,0,0,327,328,0,0,327,328,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,370,343,344,0,0,452,0,0,0,343,344,0,0,343,344,0,452,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,365,366,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,451,0,0,0,0,395,0,0,0,0,0,0,0,0,0,0,0,381,382,0,381,382,0,0,0,0,431,432,349,0,0,0,0,0,467,258,258,258,258,411,0,0,0,0,365,366,0,365,366,0,18,19,20,18,19,20,0,0,0,0,0,19,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,381,382,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,18,19,20,18,19,20,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,0,0,0,0,0,0,0,0,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,0,0,0,0,0,0,1610613812,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1650,1650,0,0,0,281,0,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,381,382,0,0,313,0,0,0,0,0,0,0,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,18,19,20,0,0,329,330,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,281,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"height":22,"id":4,"name":"furniture-above","opacity":1,"type":"tilelayer","visible":true,"width":34,"x":0,"y":0},{"data":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0,0,1,1,1,0,0,1,1,1,1,0,0,1,1,0,0,0,0,0,0,1,0,1,1,0,1,1,1,1,1,1,0,0,0,0,0,1,1,0,1,1,1,1,1,1,1,0,1,0,0,0,0,0,0,1,1,1,1,1,1,1,0,1,1,0,1,1,1,0,0,0,0,0,1,1,1,1,1,1,1,0,1,0,1,1,0,1,1,1,1,0,1,1,0,1,0,1,1,0,1,1,1,0,0,1,1,0,0,1,1,1,1,0,0,0,0,1,1,1,1,1,1,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,1,1,0,1,1,0,1,1,0,0,0,0,0,0,0,1,1,1,1,1,1,0,1,1,1,1,1,1,1,0,1,1,1,1,1,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,1,1,1,1,0,1,1,1,1,1,1,1,0,1,1,1,1,1,0,0,0,0,0,0,1,0,1,1,1,1,1,1,1,1,1,1,1,1,0,1,1,1,1,1,1,1,0,1,1,1,1,1,0,0,0,0,0,0,1,0,1,1,1,1,1,1,1,1,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,1,1,0,1,1,1,1,1,1,1,1,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,1,0,0,0,0,0,1,1,1,1,1,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,1,0,0,0,0,0,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,1,0,0,0,1,1,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,0,0,1,1,1,0,0,0,0,0,0,0,1,1,1,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,0,1,1,1,1,0,1,0,0,1,1,1,0,1,1,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,0,1,1,0,1,1,1,1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,1,1,1,1,1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,1,1,0,0,0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"height":22,"id":5,"name":"collision","opacity":1,"type":"tilelayer","visible":true,"width":34,"x":0,"y":0},{"draworder":"topdown","id":6,"name":"spawn-points","objects":[{"id":0,"name":"desk-ceo","type":"","x":48,"y":64,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-1","type":"","x":32,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-2","type":"","x":96,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-3","type":"","x":160,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-4","type":"","x":224,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-5","type":"","x":288,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"pc-6","type":"","x":352,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-team-lead","type":"","x":32,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-backend-engineer","type":"","x":96,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-product-manager","type":"","x":160,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-data-engineer","type":"","x":224,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-project-manager","type":"","x":288,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-market-researcher","type":"","x":352,"y":288,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-agent-organizer","type":"","x":320,"y":112,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"warroom-seat","type":"","x":368,"y":112,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-chief-architect","type":"","x":432,"y":80,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"desk-ui-ux-expert","type":"","x":480,"y":80,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"entrance","type":"","x":256,"y":320,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-seat-1","type":"","x":432,"y":224,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-seat-2","type":"","x":432,"y":256,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-seat-3","type":"","x":448,"y":224,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-seat-4","type":"","x":448,"y":256,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-stand-coffee","type":"","x":416,"y":320,"width":0,"height":0,"rotation":0,"visible":true,"point":true},{"id":0,"name":"cafe-stand-vending","type":"","x":464,"y":208,"width":0,"height":0,"rotation":0,"visible":true,"point":true}],"opacity":1,"type":"objectgroup","visible":true,"x":0,"y":0},{"draworder":"topdown","id":7,"name":"zones","objects":[{"id":0,"name":"boardroom","type":"","x":144,"y":48,"width":144,"height":80,"rotation":0,"visible":true},{"id":0,"name":"cafeteria","type":"","x":384,"y":192,"width":128,"height":128,"rotation":0,"visible":true}],"opacity":1,"type":"objectgroup","visible":true,"x":0,"y":0}],"nextlayerid":8,"nextobjectid":1,"orientation":"orthogonal","renderorder":"right-down","tiledversion":"1.12.0","tileheight":16,"tilesets":[{"columns":16,"firstgid":1,"image":"../tilesets/office-tileset.png","imageheight":512,"imagewidth":256,"margin":0,"name":"office-tileset","spacing":0,"tilecount":512,"tileheight":16,"tilewidth":16},{"firstgid":513,"source":"A5 Office Floors & Walls.tsx"},{"firstgid":1025,"source":"interiors.tsx"}],"tilewidth":16,"type":"map","version":"1.10","width":34}';
const brooklyn99MapRaw = '{"compressionlevel":-1,"infinite":false,"orientation":"orthogonal","renderorder":"right-down","width":36,"height":24,"tilewidth":16,"tileheight":16,"nextlayerid":99,"nextobjectid":20,"version":"1.10","tiledversion":"1.10.2","type":"map","tilesets":[{"firstgid":1,"columns":16,"image":"../tilesets/office-tileset.png","imageheight":512,"imagewidth":256,"margin":0,"name":"office-tileset","spacing":0,"tilecount":512,"tileheight":16,"tilewidth":16},{"firstgid":513,"source":"A5 Office Floors & Walls.tsx"},{"firstgid":1025,"source":"interiors.tsx"}],"layers":[{"id":1,"name":"floor","type":"tilelayer","data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,783,784,0,0,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,799,800,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,783,784,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"width":36,"height":24,"x":0,"y":0,"opacity":1,"visible":true},{"id":2,"name":"walls","type":"tilelayer","data":[570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,570,570,570,570,0,0,570,570,570,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,570,570,0,0,570,570,570,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,570,0,0,0,0,0,0,0,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,0,0,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570,570],"width":36,"height":24,"x":0,"y":0,"opacity":1,"visible":true},{"id":3,"name":"furniture-below","type":"tilelayer","data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,3,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,289,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,305,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,3,4,0,0,2,3,4,0,0,2,3,4,0,0,2,3,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,289,0,0,0,0,289,0,0,0,0,289,0,0,0,0,289,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,305,0,0,0,0,305,0,0,0,0,305,0,0,0,0,305,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,3,4,0,0,2,3,4,0,0,2,3,4,0,0,2,3,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,289,0,0,0,0,289,0,0,0,0,289,0,0,0,0,289,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,305,0,0,0,0,305,0,0,0,0,305,0,0,0,0,305,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"width":36,"height":24,"x":0,"y":0,"opacity":1,"visible":true},{"id":4,"name":"furniture-above","type":"tilelayer","data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,18,19,20,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,365,366,0,0,0,365,366,0,0,0,365,366,0,0,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,0,0,381,382,0,0,0,381,382,0,0,0,381,382,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,18,19,20,0,0,18,19,20,0,0,18,19,20,0,0,18,19,20,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,365,366,0,0,0,365,366,0,0,0,365,366,0,0,0,365,366,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,381,382,0,0,0,381,382,0,0,0,381,382,0,0,0,381,382,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,18,19,20,0,0,18,19,20,0,0,18,19,20,0,0,18,19,20,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"width":36,"height":24,"x":0,"y":0,"opacity":1,"visible":true},{"id":5,"name":"collision","type":"tilelayer","data":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,1,1,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,1,1,1,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,1,0,1,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,1,1,1,1,0,0,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,1,0,0,1,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,1,1,0,0,0,1,1,0,0,0,1,1,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,0,0,0,1,1,1,0,0,1,1,1,0,0,1,1,1,0,0,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,0,0,0,1,0,1,0,0,1,0,1,0,0,1,0,1,0,0,1,0,1,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,1,1,0,0,0,1,1,0,0,0,1,1,0,0,0,1,1,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,1,1,1,0,0,1,1,1,0,0,1,1,1,0,0,1,1,1,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,1,0,1,0,0,1,0,1,0,0,1,0,1,0,0,1,0,1,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"width":36,"height":24,"x":0,"y":0,"opacity":1,"visible":true},{"id":6,"name":"spawn-points","type":"objectgroup","objects":[{"id":1,"name":"desk-ceo","point":true,"x":496,"y":80,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":2,"name":"pc-1","point":true,"x":80,"y":208,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":3,"name":"pc-2","point":true,"x":160,"y":208,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":4,"name":"pc-3","point":true,"x":240,"y":208,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":5,"name":"pc-4","point":true,"x":320,"y":208,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":6,"name":"pc-5","point":true,"x":80,"y":304,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":7,"name":"pc-6","point":true,"x":160,"y":304,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":8,"name":"pc-7","point":true,"x":240,"y":304,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":9,"name":"pc-8","point":true,"x":320,"y":304,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":10,"name":"cafe-seat-1","point":true,"x":464,"y":224,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":11,"name":"cafe-seat-2","point":true,"x":464,"y":256,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":12,"name":"cafe-seat-3","point":true,"x":512,"y":224,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":13,"name":"cafe-seat-4","point":true,"x":512,"y":256,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":14,"name":"cafe-stand-coffee","point":true,"x":480,"y":320,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":15,"name":"cafe-stand-vending","point":true,"x":528,"y":192,"width":0,"height":0,"rotation":0,"type":"","visible":true},{"id":16,"name":"entrance","point":true,"x":272,"y":352,"width":0,"height":0,"rotation":0,"type":"","visible":true}],"draworder":"topdown","opacity":1,"visible":true,"x":0,"y":0},{"id":7,"name":"zones","type":"objectgroup","objects":[{"id":17,"name":"boardroom","x":32,"y":32,"width":128,"height":80,"rotation":0,"type":"","visible":true},{"id":18,"name":"cafeteria","x":448,"y":160,"width":112,"height":192,"rotation":0,"type":"","visible":true},{"id":19,"name":"holding","x":16,"y":288,"width":64,"height":64,"rotation":0,"type":"","visible":true}],"draworder":"topdown","opacity":1,"visible":true,"x":0,"y":0}]}';
const OFFICE_THEME = {
  id: "office",
  mapRaw: officeMapRaw,
  art: OFFICE_FLOOR,
  tilesets: [
    // office-tileset.png — embedded in the map (firstgid 1); keep the map's copy.
    { url: officeTilesetUrl, embedded: true },
    { url: a5FloorsWallsUrl, firstgid: 513, image: "a5", imagewidth: 256, imageheight: 512, tilewidth: 16, tileheight: 16, columns: 16, tilecount: 512 },
    { url: interiorsUrl, firstgid: 1025, image: "interiors", imagewidth: 256, imageheight: 1424, tilewidth: 16, tileheight: 16, columns: 16, tilecount: 1424 }
  ],
  primarySeatNames: [
    "desk-ceo",
    "pc-1",
    "pc-2",
    "pc-3",
    "pc-4",
    "pc-5",
    "pc-6",
    "desk-chief-architect",
    "desk-product-manager",
    "desk-team-lead",
    "desk-backend-engineer",
    "desk-ui-ux-expert",
    "desk-data-engineer",
    "desk-project-manager",
    "desk-market-researcher",
    "desk-agent-organizer"
  ],
  cafeSeatNames: ["cafe-seat-1", "cafe-seat-2", "cafe-seat-3", "cafe-seat-4"],
  cafeStands: [
    ["cafe-stand-coffee", "coffee"],
    ["cafe-stand-vending", "vending"]
  ],
  coffee: {
    trayTile: { x: 29, y: 15 },
    // the sideboard (counter piece)
    trayStand: { x: 29, y: 16 },
    machineStand: { x: 26, y: 20 },
    // below the counter machine
    sinkTile: { x: 28, y: 18 },
    // free counter top, right end
    sinkStand: { x: 28, y: 20 },
    maxCups: 4
  },
  anchors: {
    calendar: { x: 4, y: 1 },
    boards: { x: 6, y: 10 },
    clock: { x: 1, y: 1 }
  },
  errandSpots: [
    // plants (droplets ride on the character via startWatering)
    { kind: "water", stand: { x: 2, y: 20 }, facing: "left", fx: { x: 1, y: 20 }, duration: 4.5 },
    { kind: "water", stand: { x: 22, y: 20 }, facing: "right", fx: { x: 23, y: 20 }, duration: 4.5 },
    { kind: "water", stand: { x: 30, y: 20 }, facing: "right", fx: { x: 31, y: 20 }, duration: 4.5 },
    // the CEO office is the god's domain: its plant, window, cigar. Workers
    // never set foot in there for errands.
    { kind: "water", stand: { x: 6, y: 4 }, facing: "up", fx: { x: 6, y: 3 }, duration: 4.5, godOnly: true },
    { kind: "smoke", stand: { x: 2, y: 3 }, facing: "up", fx: { x: 2, y: 1 }, duration: 18, godOnly: true },
    { kind: "water", stand: { x: 17, y: 4 }, facing: "up", fx: { x: 17, y: 3 }, duration: 4.5 },
    // the two public wall windows — wind streaks drift into the room
    { kind: "window", stand: { x: 10, y: 3 }, facing: "up", fx: { x: 10, y: 1 }, duration: 5 },
    { kind: "window", stand: { x: 15, y: 3 }, facing: "up", fx: { x: 14, y: 1 }, duration: 5 },
    // water dispensers (hallway + the top-right corner one)
    { kind: "dispenser", stand: { x: 16, y: 3 }, facing: "down", fx: { x: 16, y: 4 }, duration: 3.5 },
    { kind: "dispenser", stand: { x: 32, y: 4 }, facing: "up", fx: { x: 32, y: 3 }, duration: 3.5 },
    // the café fridge (door light spills out) + the shelf beside it
    { kind: "fridge", stand: { x: 29, y: 20 }, facing: "up", fx: { x: 29, y: 19 }, duration: 3.2 },
    { kind: "shelf", stand: { x: 30, y: 20 }, facing: "up", fx: { x: 30, y: 18 }, duration: 4 },
    // garbage bins (entrance + café) — a paper ball arcs in
    { kind: "bin", stand: { x: 18, y: 20 }, facing: "left", fx: { x: 17, y: 20 }, duration: 2.6 },
    { kind: "bin", stand: { x: 31, y: 16 }, facing: "right", fx: { x: 32, y: 16 }, duration: 2.6 }
  ],
  monitor: {
    offTopLeftGid: 365,
    onGids: [
      [367, 0, 0],
      [368, 1, 0],
      [383, 0, 1],
      [384, 1, 1]
    ]
  },
  palette: {
    background: colors.ink[900],
    noteColors: { todo: 15916938, doing: 10406896, blocked: 15770531, done: 11067568 }
  },
  cast: {
    byName: CAST_BY_NAME,
    getFrames: (name) => getCastFrames(name),
    defaultCharacter: DEFAULT_CHARACTER
  }
};
const BROOKLYN99_THEME = {
  id: "brooklyn99",
  mapRaw: brooklyn99MapRaw,
  // PLACEHOLDER: brooklyn99.tmj uses the office gid space, so the same atlases
  // (office-tileset embedded @1, a5 @513, interiors @1025) resolve every tile.
  tilesets: OFFICE_THEME.tilesets,
  primarySeatNames: [
    "desk-ceo",
    // Captain Holt's glass office
    "pc-1",
    "pc-2",
    "pc-3",
    "pc-4",
    // bullpen — front row
    "pc-5",
    "pc-6",
    "pc-7",
    "pc-8"
    // bullpen — back row
  ],
  cafeSeatNames: ["cafe-seat-1", "cafe-seat-2", "cafe-seat-3", "cafe-seat-4"],
  cafeStands: [
    ["cafe-stand-coffee", "coffee"],
    ["cafe-stand-vending", "vending"]
  ],
  coffee: {
    trayTile: { x: 33, y: 18 },
    trayStand: { x: 33, y: 19 },
    machineStand: { x: 30, y: 21 },
    sinkTile: { x: 31, y: 18 },
    sinkStand: { x: 31, y: 19 },
    maxCups: 4
  },
  anchors: {
    calendar: { x: 4, y: 1 },
    // briefing-room top wall → TRIGGERS
    boards: { x: 14, y: 1 },
    // over the bullpen → TASKS
    clock: { x: 1, y: 1 }
    // top-left corner → CLOSING TIME
  },
  // Placeholder errand anchors authored to brooklyn99.tmj's open floor (verified
  // walkable against the map's collision layer + desk stamps). The godOnly spots
  // sit inside Holt's glass office.
  errandSpots: [
    // public plants around the bullpen
    { kind: "water", stand: { x: 2, y: 13 }, facing: "left", fx: { x: 1, y: 13 }, duration: 4.5 },
    { kind: "water", stand: { x: 24, y: 15 }, facing: "right", fx: { x: 25, y: 15 }, duration: 4.5 },
    { kind: "water", stand: { x: 13, y: 15 }, facing: "down", fx: { x: 13, y: 16 }, duration: 4.5 },
    // Captain Holt's glass office — god's domain (plant + cigar at the window)
    { kind: "water", stand: { x: 28, y: 6 }, facing: "up", fx: { x: 28, y: 5 }, duration: 4.5, godOnly: true },
    { kind: "smoke", stand: { x: 34, y: 2 }, facing: "up", fx: { x: 34, y: 0 }, duration: 18, godOnly: true },
    // public windows on the north wall — wind streaks drift in
    { kind: "window", stand: { x: 14, y: 1 }, facing: "up", fx: { x: 14, y: 0 }, duration: 5 },
    { kind: "window", stand: { x: 22, y: 1 }, facing: "up", fx: { x: 22, y: 0 }, duration: 5 },
    // water dispensers (bullpen + entrance corridor)
    { kind: "dispenser", stand: { x: 8, y: 15 }, facing: "down", fx: { x: 8, y: 16 }, duration: 3.5 },
    { kind: "dispenser", stand: { x: 17, y: 20 }, facing: "down", fx: { x: 17, y: 21 }, duration: 3.5 },
    // break-room fridge + shelf (by the coffee economy)
    { kind: "fridge", stand: { x: 29, y: 21 }, facing: "up", fx: { x: 29, y: 20 }, duration: 3.2 },
    { kind: "shelf", stand: { x: 34, y: 18 }, facing: "up", fx: { x: 34, y: 17 }, duration: 4 },
    // garbage bins (entrance + break room)
    { kind: "bin", stand: { x: 19, y: 20 }, facing: "left", fx: { x: 18, y: 20 }, duration: 2.6 },
    { kind: "bin", stand: { x: 34, y: 15 }, facing: "up", fx: { x: 34, y: 14 }, duration: 2.6 }
  ],
  // PLACEHOLDER: brooklyn99.tmj paints the office desk stamp (monitor gid 365).
  monitor: OFFICE_THEME.monitor,
  // PLACEHOLDER: office palette + cast until Pam's B99 art (§C/§D) lands.
  palette: OFFICE_THEME.palette,
  cast: OFFICE_THEME.cast
};
const THEMES = {
  office: OFFICE_THEME,
  brooklyn99: BROOKLYN99_THEME
};
function getTheme(id) {
  return THEMES[id] ?? OFFICE_THEME;
}
function resolveThemeMap(theme) {
  const m = JSON.parse(theme.mapRaw);
  return {
    ...m,
    tilesets: theme.tilesets.map((t, i) => {
      if (t.embedded) return m.tilesets[i];
      const { url: _url, embedded: _embedded, ...meta } = t;
      return meta;
    })
  };
}
function themeTilesetUrls(theme) {
  return theme.tilesets.map((t) => t.url);
}
function isThemeRenderable(theme) {
  try {
    const m = JSON.parse(theme.mapRaw);
    return typeof m.width === "number" && m.width > 0 && typeof m.height === "number" && m.height > 0 && Array.isArray(m.layers) && Array.isArray(m.tilesets);
  } catch {
    return false;
  }
}
async function recoloredTileset(url, colors2) {
  const img = new Image();
  img.src = url;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const g = canvas.getContext("2d");
  if (!g) throw new Error("no 2d context");
  g.drawImage(img, 0, 0);
  const d = g.getImageData(0, 0, canvas.width, canvas.height);
  recolorPixels(d.data, colors2);
  g.putImageData(d, 0, 0);
  return canvas.toDataURL("image/png");
}
async function officeWithChanges(id) {
  const overlay = await readThemeOverlay(id);
  if (!overlay.ok) throw new Error(`${overlay.refusal}: ${overlay.detail}`);
  const { manifest: m, pictures } = overlay;
  const map = applyLayoutToOfficeMap(JSON.parse(OFFICE_THEME.mapRaw), m.layout);
  const colors2 = m.colors;
  return {
    ...OFFICE_THEME,
    mapRaw: JSON.stringify(map),
    primarySeatNames: officeSeatsFor(OFFICE_THEME.primarySeatNames, m.layout?.seats),
    palette: { ...OFFICE_THEME.palette, ...colors2?.background ? { background: parseInt(colors2.background.slice(1), 16) } : {} },
    tilesets: colors2 && (colors2.shift || colors2.swap) ? await Promise.all(OFFICE_THEME.tilesets.map(async (t) => ({ ...t, url: await recoloredTileset(t.url, colors2) }))) : OFFICE_THEME.tilesets,
    placed: (m.images ?? []).map((img) => ({ url: pictures[img.file], x: img.x, y: img.y, depth: img.depth }))
  };
}
async function loadTheme(id, onCannotUse) {
  if (officeThemeBaseOf(id) === "office") {
    try {
      return await officeWithChanges(id);
    } catch (e) {
      console.warn(`[themeLoader] theme '${id}' cannot be used; showing the office:`, e);
      onCannotUse?.(id);
      return OFFICE_THEME;
    }
  }
  const theme = getTheme(id);
  if (!isThemeRenderable(theme)) {
    console.warn(`[themeLoader] theme '${id}' is not renderable — falling back to 'office'`);
    return OFFICE_THEME;
  }
  return theme;
}
const MAX_DESK_PUMPKINS = 5;
const MAX_FLOOR_PUMPKINS = 4;
const MAX_COBWEBS = 6;
const BAT_COUNT = 3;
const MAX_RED_LAMPS = 5;
const LAMP_GAP = 3;
function tileNoise(x, y) {
  let h = x * 374761393 + y * 668265263 | 0;
  h = Math.imul(h ^ h >>> 13, 1274126177);
  return ((h ^ h >>> 16) >>> 0) / 4294967296;
}
function spread(all, max) {
  if (all.length <= max) return [...all];
  const kept = [all[0]];
  while (kept.length < max) {
    let best = null;
    let bestD = -1;
    for (const c of all) {
      if (kept.includes(c)) continue;
      let d = Infinity;
      for (const k of kept) d = Math.min(d, (c.x - k.x) ** 2 + (c.y - k.y) ** 2);
      if (d > bestD) {
        bestD = d;
        best = c;
      }
    }
    if (!best) break;
    kept.push(best);
  }
  return kept;
}
function wallRun(input, x, y) {
  let n = 0;
  while (n < 3 && input.wallAt(x, y - 1 - n) !== 0) n++;
  return n;
}
function planHalloween(input) {
  const ts = input.tileSize;
  const avoided = new Set(input.avoid.map((t) => `${t.x},${t.y}`));
  const desks = input.desks.filter((_, i) => i % 2 === 1);
  const pumpkins = spread(desks, MAX_DESK_PUMPKINS).map((d) => ({
    size: "desk",
    x: d.x * ts - 9,
    y: d.y * ts + 27,
    // One under the monitor overlay's own key, so a lit screen never hides it.
    z: (d.y + 2) * ts - 2,
    phase: tileNoise(d.x, d.y)
  }));
  const corners = [];
  for (let y = 0; y < input.height; y++) {
    for (let x = 0; x < input.width; x++) {
      if (!input.isWalkable(x, y) || avoided.has(`${x},${y}`)) continue;
      if (y + 1 >= input.height || input.isWalkable(x, y + 1)) continue;
      if (!input.isWalkable(x, y - 1)) continue;
      if (input.isWalkable(x - 1, y) === input.isWalkable(x + 1, y)) continue;
      corners.push({ x, y });
    }
  }
  for (const c of spread(corners, MAX_FLOOR_PUMPKINS)) {
    pumpkins.push({
      size: "floor",
      x: c.x * ts + 2,
      y: c.y * ts + ts - 1,
      z: (c.y + 1) * ts,
      phase: tileNoise(c.x, c.y)
    });
  }
  const webTiles = [];
  for (let y = 1; y < input.height; y++) {
    for (let x = 0; x < input.width; x++) {
      if (input.wallAt(x, y) !== 0 || input.wallAt(x, y - 1) === 0) continue;
      const run = wallRun(input, x, y);
      const row = y - Math.max(1, run === 3 ? 2 : run);
      for (const side of [-1, 1]) {
        const beside = input.wallAt(x + side, y);
        if (beside === 0 || beside !== input.wallAt(x + side, row)) continue;
        webTiles.push({ x, y, row, flip: side === 1 });
        break;
      }
    }
  }
  const cobwebs = spread(webTiles, MAX_COBWEBS).map((w) => ({ x: (w.flip ? w.x + 1 : w.x) * ts, y: w.row * ts, flip: w.flip }));
  const decorAt = input.decorAt ?? (() => false);
  const lampTiles = [];
  for (let y = 0; y + 1 < input.height; y++) {
    for (let x = 0; x < input.width; x++) {
      if (input.wallAt(x, y) === 0 || input.wallAt(x, y + 1) !== 0) continue;
      if (input.wallAt(x - 1, y) === 0 || input.wallAt(x + 1, y) === 0) continue;
      if (decorAt(x, y) || decorAt(x, y - 1)) continue;
      if (webTiles.some((w) => Math.abs(w.x - x) <= 1 && Math.abs(w.y - y) <= 2)) continue;
      lampTiles.push({ x, y });
    }
  }
  const lampPick = [];
  for (const t of spread(lampTiles, MAX_RED_LAMPS)) {
    if (lampPick.every((k) => Math.max(Math.abs(k.x - t.x), Math.abs(k.y - t.y)) >= LAMP_GAP)) lampPick.push(t);
  }
  const lamps = lampPick.map((t) => ({
    x: t.x * ts + ts / 2,
    y: t.y * ts + 6,
    z: (t.y + 1) * ts,
    phase: tileNoise(t.x, t.y)
  }));
  const cx = Math.floor((input.machineStand.x + input.sinkTile.x) / 2);
  const candy = {
    x: cx * ts + 10,
    y: input.sinkTile.y * ts + 13,
    z: (input.sinkTile.y + 1) * ts
  };
  return { pumpkins, cobwebs, candy, bats: BAT_COUNT, lamps };
}
const ORANGE = 16095804;
const ORANGE_DARK = 15040290;
const ORANGE_LIGHT = 16761466;
const STEM = 8365632;
const FLAME = 16769162;
const PURPLE = 8081328;
const NIGHT = 2826808;
const WEB = 7299717;
const DUSK = 2826808;
const CANDY = [PURPLE, ORANGE, STEM, 16052474];
const LAMP_RED = 13120046;
const LAMP_RIM = 16738906;
const GLOW_RINGS = 10;
const DUSK_ALPHA = 0.4;
const LAMP_RADIUS = 22;
class SeasonDecor {
  constructor(opts) {
    this.opts = opts;
    const { plan, charLayer, root, mapWidth, mapHeight, tintInset = 0 } = opts;
    this.tint.rect(tintInset, 0, mapWidth - tintInset * 2, mapHeight - tintInset).fill({ color: DUSK, alpha: 1 });
    this.tint.alpha = DUSK_ALPHA;
    this.tint.eventMode = "none";
    root.addChildAt(this.tint, root.getChildIndex(charLayer));
    for (const web of plan.cobwebs) {
      const g = new Graphics();
      g.eventMode = "none";
      paintCobweb(g);
      g.position.set(web.x, web.y);
      if (web.flip) g.scale.x = -1;
      g.zIndex = web.y + 1;
      charLayer.addChild(g);
      this.parts.push(g);
    }
    for (const p of plan.pumpkins) {
      const body = new Graphics();
      body.eventMode = "none";
      const face = new Graphics();
      face.eventMode = "none";
      paintPumpkin(body, p.size);
      const holder = new Container();
      holder.eventMode = "none";
      holder.addChild(body, face);
      holder.position.set(p.x, p.y);
      holder.zIndex = p.z;
      charLayer.addChild(holder);
      this.parts.push(holder);
      const glow = new Graphics();
      glow.eventMode = "none";
      glow.blendMode = "add";
      const w = p.size === "floor" ? 12 : 8;
      glow.position.set(p.x + w / 2, p.y - (p.size === "floor" ? 5 : 3));
      this.glows.addChild(glow);
      this.lanterns.push({ plan: p, glow, face });
    }
    this.reds.eventMode = "none";
    for (const l of plan.lamps) {
      const bulb = new Graphics();
      bulb.eventMode = "none";
      paintLamp(bulb);
      bulb.position.set(l.x, l.y);
      bulb.zIndex = l.z;
      charLayer.addChild(bulb);
      this.parts.push(bulb);
      this.bulbs.push(bulb);
      const glow = new Graphics();
      glow.eventMode = "none";
      glow.blendMode = "add";
      for (let i = 0; i < GLOW_RINGS; i++) {
        const k = 1 - i / GLOW_RINGS;
        glow.circle(0, 0, LAMP_RADIUS * k).fill({ color: i < GLOW_RINGS / 2 ? LAMP_RIM : LAMP_RED, alpha: 0.04 });
      }
      glow.position.set(l.x, l.y);
      this.reds.addChild(glow);
      this.lamps.push({ plan: l, glow });
    }
    this.glows.addChild(this.reds);
    const bowl = new Graphics();
    bowl.eventMode = "none";
    paintCandyBowl(bowl);
    bowl.position.set(plan.candy.x, plan.candy.y);
    bowl.zIndex = plan.candy.z;
    charLayer.addChild(bowl);
    this.parts.push(bowl);
    this.glows.eventMode = "none";
    this.sky.eventMode = "none";
    root.addChild(this.glows, this.sky);
    if (!opts.reducedMotion) {
      for (let i = 0; i < plan.bats; i++) {
        const g = new Graphics();
        g.eventMode = "none";
        g.visible = false;
        this.sky.addChild(g);
        this.bats.push({ g, wait: 2 + i * 7, t: 0, flying: false, fromX: 0, toX: 0, baseY: 0, swing: 0, duration: 1 });
      }
    }
    this.parts.push(this.tint, this.glows, this.sky);
    this.paintLanterns(0);
    this.setOn(false);
  }
  tint = new Graphics();
  props = new Container();
  glows = new Container();
  sky = new Container();
  lanterns = [];
  lamps = [];
  /** The red lamps' bulbs sit in the sorted layer; their glows in `reds`. */
  bulbs = [];
  reds = new Container();
  bats = [];
  parts = [];
  on = false;
  t = 0;
  /** Dress the floor, or put it back exactly as it was. */
  setOn(on) {
    this.on = on;
    for (const p of this.parts) p.visible = on;
    this.showLamps();
  }
  /** The red lamps are lit whenever the decor is on. */
  showLamps() {
    const lit = this.on;
    this.reds.visible = lit;
    for (const b of this.bulbs) b.visible = lit;
    if (lit) this.flickerLamps(this.opts.reducedMotion ? 0 : this.t);
  }
  isOn() {
    return this.on;
  }
  update(dt) {
    if (!this.on) return;
    this.t += dt;
    if (!this.opts.reducedMotion) this.paintLanterns(this.t);
    if (!this.opts.reducedMotion) this.flickerLamps(this.t);
    for (let i = 0; i < this.bats.length; i++) this.updateBat(this.bats[i], i, dt);
  }
  destroy() {
    for (const p of this.parts) {
      p.parent?.removeChild(p);
      p.destroy({ children: true });
    }
    this.parts.length = 0;
    this.lanterns.length = 0;
    this.lamps.length = 0;
    this.bulbs.length = 0;
    this.bats.length = 0;
  }
  paintLanterns(t) {
    for (const l of this.lanterns) {
      const flicker = candle(t, l.plan.phase);
      const floor = l.plan.size === "floor";
      const r = floor ? 15 : 10;
      l.glow.clear();
      for (let i = 0; i < GLOW_RINGS; i++) {
        const k = 1 - i / GLOW_RINGS;
        l.glow.circle(0, 0, r * k).fill({ color: i < GLOW_RINGS / 2 ? ORANGE : FLAME, alpha: 0.022 * flicker });
      }
      l.face.clear();
      paintFace(l.face, l.plan.size, 0.75 + 0.25 * flicker);
    }
  }
  /** The red lamps flicker like the lanterns, through opacity alone. */
  flickerLamps(t) {
    for (const l of this.lamps) l.glow.alpha = candle(t, l.plan.phase);
  }
  updateBat(b, index, dt) {
    if (!b.flying) {
      b.wait -= dt;
      if (b.wait > 0) return;
      const { mapWidth, mapHeight } = this.opts;
      const leftToRight = (Math.floor(this.t * 7) + index) % 2 === 0;
      b.fromX = leftToRight ? -12 : mapWidth + 12;
      b.toX = leftToRight ? mapWidth + 12 : -12;
      b.baseY = mapHeight * (0.12 + 0.55 * ((this.t * 0.37 + index * 0.31) % 1));
      b.swing = 10 + 8 * ((index + 1) % 3);
      b.duration = 6.5 + index * 1.2;
      b.t = 0;
      b.flying = true;
      b.g.visible = true;
      b.g.scale.x = leftToRight ? 1 : -1;
    }
    b.t += dt;
    const k = b.t / b.duration;
    if (k >= 1) {
      b.flying = false;
      b.g.visible = false;
      b.wait = 14 + index * 6 + this.t * 3 % 9;
      return;
    }
    b.g.position.set(
      b.fromX + (b.toX - b.fromX) * k,
      b.baseY + Math.sin(k * Math.PI * 3 + index) * b.swing
    );
    paintBat(b.g, Math.floor(b.t * 9) % 2 === 0);
  }
}
function candle(t, phase) {
  const ph = phase * Math.PI * 2;
  return 0.72 + 0.14 * Math.sin(t * 2.1 + ph) + 0.09 * Math.sin(t * 5.3 + ph * 1.7) + 0.05 * Math.sin(t * 13 + ph * 0.6);
}
function paintLamp(g) {
  g.rect(-2, -4, 5, 1).fill(NIGHT);
  g.rect(0, -3, 1, 1).fill(NIGHT);
  g.roundRect(-2, -2, 5, 4, 1).fill(LAMP_RED);
  g.rect(-1, -1, 2, 1).fill(LAMP_RIM);
}
function paintCobweb(g) {
  const spokes = [[15, 0], [13, 6], [9, 10], [5, 13], [0, 15]];
  for (const [x, y] of spokes) {
    g.moveTo(0, 0).lineTo(x, y).stroke({ color: WEB, width: 1, alpha: 0.8, pixelLine: true });
  }
  for (const k of [0.4, 0.7, 1]) {
    g.moveTo(spokes[0][0] * k, spokes[0][1] * k);
    for (let i = 1; i < spokes.length; i++) {
      const [ax, ay] = spokes[i - 1];
      const [bx, by] = spokes[i];
      g.quadraticCurveTo((ax + bx) * k * 0.44, (ay + by) * k * 0.44, bx * k, by * k);
    }
    g.stroke({ color: WEB, width: 1, alpha: 0.6, pixelLine: true });
  }
}
function paintPumpkin(g, size) {
  if (size === "floor") {
    g.ellipse(6, 0, 6, 1.5).fill({ color: NIGHT, alpha: 0.35 });
    g.roundRect(0, -9, 12, 9, 3).fill(ORANGE);
    g.roundRect(0, -9, 3, 9, 2).fill(ORANGE_DARK);
    g.roundRect(9, -9, 3, 9, 2).fill(ORANGE_DARK);
    g.rect(4, -9, 1, 9).fill({ color: ORANGE_DARK, alpha: 0.55 });
    g.rect(7, -9, 1, 9).fill({ color: ORANGE_DARK, alpha: 0.55 });
    g.rect(2, -8, 3, 1).fill(ORANGE_LIGHT);
    g.rect(5, -12, 2, 3).fill(STEM);
    g.rect(7, -12, 2, 1).fill(STEM);
    return;
  }
  g.ellipse(4, 0, 4, 1).fill({ color: NIGHT, alpha: 0.3 });
  g.roundRect(0, -6, 8, 6, 2).fill(ORANGE);
  g.roundRect(0, -6, 2, 6, 1).fill(ORANGE_DARK);
  g.roundRect(6, -6, 2, 6, 1).fill(ORANGE_DARK);
  g.rect(2, -5, 2, 1).fill(ORANGE_LIGHT);
  g.rect(3, -8, 2, 2).fill(STEM);
}
function paintFace(g, size, light) {
  const fill = { color: FLAME, alpha: light };
  if (size === "floor") {
    g.rect(2, -7, 2, 2).fill(fill);
    g.rect(8, -7, 2, 2).fill(fill);
    g.rect(5, -5, 2, 1).fill(fill);
    g.rect(2, -3, 8, 1).fill(fill);
    g.rect(3, -2, 2, 1).fill(fill);
    g.rect(7, -2, 2, 1).fill(fill);
    return;
  }
  g.rect(1, -5, 2, 1).fill(fill);
  g.rect(5, -5, 2, 1).fill(fill);
  g.rect(2, -3, 4, 1).fill(fill);
}
function paintCandyBowl(g) {
  const sweets = [[0, -6], [3, -7], [6, -6], [2, -5], [4, -5]];
  sweets.forEach(([x, y], i) => g.rect(x, y, 2, 2).fill(CANDY[i % CANDY.length]));
  g.roundRect(0, -4, 8, 4, 2).fill(NIGHT);
  g.rect(1, -4, 6, 1).fill(PURPLE);
  g.rect(2, -2, 1, 1).fill(ORANGE);
  g.rect(5, -2, 1, 1).fill(ORANGE);
}
function paintBat(g, wingsUp) {
  g.clear();
  g.rect(-1, -1, 3, 3).fill(NIGHT);
  g.rect(-1, -2, 1, 1).fill(NIGHT);
  g.rect(1, -2, 1, 1).fill(NIGHT);
  const y = wingsUp ? -3 : 0;
  const tip = wingsUp ? -1 : 2;
  g.poly([-1, 0, -4, y, -7, tip, -4, 1]).fill(NIGHT);
  g.poly([2, 0, 5, y, 8, tip, 5, 1]).fill(NIGHT);
}
const CHEER_MIN_BUSY_MS = 6e4;
const ERRAND_THOUGHTS = {
  water: ["office.errand.water.0", "office.errand.water.1", "office.errand.water.2"],
  window: ["office.errand.window.0", "office.errand.window.1", "office.errand.window.2"],
  dispenser: ["office.errand.dispenser.0", "office.errand.dispenser.1", "office.errand.dispenser.2"],
  fridge: ["office.errand.fridge.0", "office.errand.fridge.1", "office.errand.fridge.2"],
  shelf: ["office.errand.shelf.0", "office.errand.shelf.1", "office.errand.shelf.2"],
  bin: ["office.errand.bin.0", "office.errand.bin.1", "office.errand.bin.2"],
  smoke: ["office.errand.smoke.0", "office.errand.smoke.1", "office.errand.smoke.2", "office.errand.smoke.3"]
};
const SUCK_UP_KEYS = [
  "office.suckUp.0",
  "office.suckUp.1",
  "office.suckUp.2",
  "office.suckUp.3",
  "office.suckUp.4",
  "office.suckUp.5",
  "office.suckUp.6"
];
const GOSSIP_KEYS = [
  "office.gossip.0",
  "office.gossip.1",
  "office.gossip.2",
  "office.gossip.3",
  "office.gossip.4",
  "office.gossip.5",
  "office.gossip.6"
];
const CHEER_KEYS = [
  "office.cheer.0",
  "office.cheer.1",
  "office.cheer.2",
  "office.cheer.3",
  "office.cheer.4",
  "office.cheer.5",
  "office.cheer.6"
];
function loadTexture(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const tex = Texture.from(img);
      tex.source.scaleMode = "nearest";
      resolve(tex);
    };
    img.onerror = () => reject(new Error("failed to load " + url.slice(0, 40)));
    img.src = url;
  });
}
function liveActivity(agent, fallback, t) {
  const action = (agent.action || "").trim();
  const word = floorActionWord(action);
  if (word) return t(word.key, word.vars);
  if (action) return action;
  return firstWords(agent.lastPrompt) || fallback;
}
function firstWords(prompt, maxWords = 6, maxChars = 42) {
  if (!prompt) return "";
  const words = prompt.trim().split(/\s+/);
  let out = words.slice(0, maxWords).join(" ");
  const truncatedWords = words.length > maxWords;
  if (out.length > maxChars) out = out.slice(0, maxChars).trimEnd();
  else if (truncatedWords) out += "…";
  return out;
}
function OfficeFloor({ onPage, onCannotDraw, onFallback, onSpots } = {}) {
  const { t } = useTranslation();
  const officeTheme = useStore((s) => s.officeTheme);
  const wantsPack = isStationPack(officeTheme);
  const [state, setState] = reactExports.useState(null);
  reactExports.useEffect(() => {
    if (!wantsPack) {
      setState(null);
      return;
    }
    let live = true;
    setState(null);
    loadStationPack(officeTheme).then(
      ({ pack, diagnostics }) => {
        if (!live) return;
        if (!pack) console.warn(`[OfficeFloor] theme "${officeTheme}" cannot be used; showing the office:
` + diagnostics.filter((d) => d.level === "error").map((d) => "  " + d.message).join("\n"));
        setState({ id: officeTheme, pack, failed: !pack });
      },
      (e) => {
        if (!live) return;
        console.warn(`[OfficeFloor] theme "${officeTheme}" could not be loaded; showing the office:`, e);
        setState({ id: officeTheme, pack: null, failed: true });
      }
    );
    return () => {
      live = false;
    };
  }, [officeTheme, wantsPack]);
  const [sceneFailed, setSceneFailed] = reactExports.useState(null);
  reactExports.useEffect(() => {
    setSceneFailed(null);
  }, [officeTheme]);
  const failedId = state?.failed ? state.id : sceneFailed === officeTheme ? officeTheme : null;
  const [failedName, setFailedName] = reactExports.useState(null);
  reactExports.useEffect(() => {
    if (!failedId) return;
    let live = true;
    void savedThemeName(failedId).then((label) => {
      if (live) setFailedName({ id: failedId, label });
    });
    return () => {
      live = false;
    };
  }, [failedId]);
  const failed = failedId && failedName?.id === failedId ? failedName : null;
  const name = failed ? failed.label ?? t("pro.home.floor.yourTheme") : null;
  const named = !!failed?.label;
  reactExports.useEffect(() => {
    onFallback?.(name, named);
  }, [name, named, onFallback]);
  if (wantsPack && (!state || state.id !== officeTheme)) return /* @__PURE__ */ jsxRuntimeExports.jsx("div", { style: { width: "100%", height: "100%", background: hex(colors.ink[900]) } });
  if (state?.pack) return /* @__PURE__ */ jsxRuntimeExports.jsx(StationFloor, { pack: state.pack, onSpots, dataFeed: openFloorDataFeed });
  return /* @__PURE__ */ jsxRuntimeExports.jsxs("div", { style: { position: "relative", width: "100%", height: "100%" }, children: [
    /* @__PURE__ */ jsxRuntimeExports.jsx(OfficeScene, { onPage, onCannotDraw, onSpots, onThemeFailed: setSceneFailed }),
    name && !onFallback && /* @__PURE__ */ jsxRuntimeExports.jsx("div", { role: "status", style: { position: "absolute", left: 8, bottom: 8, right: 8, padding: "6px 10px", background: "rgba(20,16,24,0.85)", color: "#ffd0b5", fontFamily: "monospace", fontSize: 12, pointerEvents: "none" }, children: t("stationFloor.packFailed", { name }) })
  ] });
}
function OfficeScene({ onPage, onCannotDraw, onSpots, onThemeFailed }) {
  const cannotDrawRef = reactExports.useRef(onCannotDraw);
  cannotDrawRef.current = onCannotDraw;
  const spotsRef = reactExports.useRef(onSpots);
  spotsRef.current = onSpots;
  const themeFailedRef = reactExports.useRef(onThemeFailed);
  themeFailedRef.current = onThemeFailed;
  const { t, i18n } = useTranslation();
  const activity = (a, fallback = "") => liveActivity(a, fallback, t);
  const hostRef = reactExports.useRef(null);
  const [shape, setShape] = reactExports.useState({ w: OFFICE_FLOOR.w, h: OFFICE_FLOOR.h });
  const [fill, setFill] = reactExports.useState(hex(colors.ink[900]));
  const offsetRef = reactExports.useRef({ x: 0, y: 0 });
  const appRef = reactExports.useRef(null);
  const mountIdRef = reactExports.useRef(0);
  const [glGeneration, setGlGeneration] = reactExports.useState(0);
  const initRetriesRef = reactExports.useRef(0);
  const officeTheme = useStore((s) => s.officeTheme);
  const fullscreenAgentId = useStore((s) => s.fullscreenAgentId);
  const ideOpen = useStore((s) => s.ideOpen);
  const [docHidden, setDocHidden] = reactExports.useState(() => document.hidden);
  reactExports.useEffect(() => {
    const onVis = () => setDocHidden(document.hidden);
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, []);
  const [windowHidden, setWindowHidden] = reactExports.useState(false);
  reactExports.useEffect(() => {
    let heard = false;
    const off = window.cth.onWindowVisibility?.((v) => {
      heard = true;
      setWindowHidden(v.hidden === true);
    });
    void window.cth.getWindowVisibility?.().then((v) => {
      if (!heard) setWindowHidden(v?.hidden === true);
    }).catch(() => {
    });
    return () => {
      off?.();
    };
  }, []);
  reactExports.useEffect(() => {
    if (!windowHidden) return;
    const onScreen = () => setWindowHidden(false);
    window.addEventListener("pointermove", onScreen, true);
    window.addEventListener("keydown", onScreen, true);
    window.addEventListener("resize", onScreen, true);
    return () => {
      window.removeEventListener("pointermove", onScreen, true);
      window.removeEventListener("keydown", onScreen, true);
      window.removeEventListener("resize", onScreen, true);
    };
  }, [windowHidden]);
  const paused = floorPaused({ fullscreen: !!fullscreenAgentId, ideOpen, docHidden, windowHidden });
  const pausedRef = reactExports.useRef(paused);
  const taskBoardPollRef = reactExports.useRef(null);
  reactExports.useEffect(() => {
    pausedRef.current = paused;
    applyFloorPause([appRef.current?.ticker, Ticker.system], paused);
    if (!paused) taskBoardPollRef.current?.();
  }, [paused]);
  reactExports.useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    while (host.firstChild) host.removeChild(host.firstChild);
    const mountId = ++mountIdRef.current;
    const app = new Application();
    appRef.current = app;
    const runtimes = /* @__PURE__ */ new Map();
    const seatClaims = /* @__PURE__ */ new Set();
    const pendingAdds = /* @__PURE__ */ new Set();
    const envelopes = [];
    const MAX_ENVELOPES = 16;
    const init = async () => {
      const theme = await loadTheme(officeTheme, (id) => themeFailedRef.current(id));
      await app.init({
        // No colour at all on Home: Pixi clears to it unmultiplied, so even at alpha 0 it lightens the page behind.
        ...onPage ? { backgroundAlpha: 0 } : { background: hexNum(theme.palette.background) },
        antialias: false,
        roundPixels: true,
        // resolution: 1 let the OS/browser upscale the canvas on scaled and
        // HiDPI displays (125–150% is the Windows laptop default), blurring
        // everything — worst of all the bubble text, which is small to begin
        // with. Render at the real device pixel density instead, floored at 2
        // so the half-scale-supersampled bubble text stays legible even at
        // 100% scaling. autoDensity keeps the canvas CSS size in logical px.
        resolution: Math.max(window.devicePixelRatio || 1, 2),
        autoDensity: true,
        width: host.clientWidth || 800,
        height: host.clientHeight || 600,
        // Graphics on GPU: ask for the faster graphics card (the mode this run started with).
        ...await floorPowerPreference()
      });
      if (mountIdRef.current !== mountId) {
        safeDestroy(app);
        return;
      }
      while (host.firstChild) host.removeChild(host.firstChild);
      host.appendChild(app.canvas);
      recordFloorRenderer(app);
      app.__glRecovery = installContextLossRecovery(app.canvas, {
        onRebuild: () => {
          if (mountIdRef.current === mountId) setGlGeneration((n) => n + 1);
        },
        onGiveUp: () => {
          if (mountIdRef.current !== mountId) return;
          cannotDrawRef.current?.();
          host.appendChild(floorNote(t("office.gpuError")));
        }
      });
      const tilesetTextures = await Promise.all(
        themeTilesetUrls(theme).map(loadTexture)
      );
      if (mountIdRef.current !== mountId) {
        safeDestroy(app);
        return;
      }
      const world = new Container();
      app.stage.addChild(world);
      const mapRenderer = new TiledMapRenderer(resolveThemeMap(theme), tilesetTextures);
      world.addChild(mapRenderer.getContainer());
      const charLayer = mapRenderer.getCharacterContainer();
      for (const p of theme.placed ?? []) {
        try {
          const sprite = new Sprite(await loadTexture(p.url));
          sprite.position.set(p.x, p.y);
          const root = mapRenderer.getContainer();
          if (p.depth === "below") root.addChildAt(sprite, root.getChildIndex(charLayer));
          else if (p.depth === "above") root.addChild(sprite);
          else {
            sprite.zIndex = p.y + sprite.height;
            charLayer.addChild(sprite);
          }
        } catch (e) {
          console.warn("[OfficeFloor] a picture of the theme could not be drawn:", e);
        }
      }
      if (mountIdRef.current !== mountId) {
        safeDestroy(app);
        return;
      }
      const tileCount = mapRenderer.getContainer().children.reduce(
        (n, c) => n + (c.children?.length ?? 0),
        0
      );
      console.log(`[OfficeFloor] map ${mapRenderer.width}x${mapRenderer.height}, ${tileCount} tile sprites rendered`);
      const camera = new Camera(world);
      camera.setMapSize(mapRenderer.width * mapRenderer.tileSize, mapRenderer.height * mapRenderer.tileSize);
      if (theme.art) camera.setArt(theme.art.x, theme.art.y, theme.art.w, theme.art.h);
      if (mountIdRef.current === mountId) {
        setShape(theme.art ? { w: theme.art.w, h: theme.art.h } : { w: mapRenderer.width * mapRenderer.tileSize, h: mapRenderer.height * mapRenderer.tileSize });
        setFill(hex(theme.palette.background));
      }
      camera.setViewSize(app.screen.width, app.screen.height);
      camera.fitToScreen();
      const calTs = mapRenderer.tileSize;
      const calG = new Graphics();
      calG.eventMode = "static";
      calG.cursor = "pointer";
      calG.position.set(theme.anchors.calendar.x * calTs + 8, theme.anchors.calendar.y * calTs + 5);
      calG.zIndex = 3 * calTs;
      calG.on("pointertap", (ev) => {
        ev.stopPropagation();
        const st = useStore.getState();
        const god = st.agents.find((a) => a.isGod);
        if (god) st.select(god.id);
        st.requestCommandCenterTab("triggers");
      });
      calG.rect(7, -2, 2, 2).fill(4864850);
      calG.rect(0, 0, 16, 20).fill(4864850);
      calG.rect(1, 1, 14, 18).fill(15919832);
      calG.rect(1, 1, 14, 4).fill(13193039);
      calG.rect(4, 0, 1, 2).fill(14209988);
      calG.rect(11, 0, 1, 2).fill(14209988);
      for (let r = 0; r < 3; r++) {
        for (let c = 0; c < 5; c++) {
          calG.rect(2 + c * 3, 7 + r * 4, 2, 2).fill(12102544);
        }
      }
      calG.rect(8, 11, 2, 2).fill(13193039);
      charLayer.addChild(calG);
      const seatTiles = [];
      const seatSeen = /* @__PURE__ */ new Set();
      const addSeat = (t2) => {
        if (!t2) return;
        const k = `${t2.x},${t2.y}`;
        if (seatSeen.has(k)) return;
        seatSeen.add(k);
        seatTiles.push({ x: t2.x, y: t2.y });
      };
      for (const name of theme.primarySeatNames) addSeat(mapRenderer.getSpawnPoint(name));
      const addZoneSeats = (zone) => {
        const z = mapRenderer.getZone(zone);
        if (!z) return;
        for (let y = z.y; y < z.y + z.height; y++) {
          for (let x = z.x; x < z.x + z.width; x++) {
            if (mapRenderer.isWalkable(x, y)) addSeat({ x, y });
          }
        }
      };
      addZoneSeats("boardroom");
      const entrance = mapRenderer.getSpawnPoint("entrance") ?? { x: Math.floor(mapRenderer.width / 2), y: mapRenderer.height - 2 };
      const waitTiles = [];
      const waitSeen = /* @__PURE__ */ new Set();
      for (let radius = 0; radius <= 6 && waitTiles.length < 16; radius++) {
        for (let dy = -radius; dy <= radius; dy++) {
          for (let dx = -radius; dx <= radius; dx++) {
            if (Math.max(Math.abs(dx), Math.abs(dy)) !== radius) continue;
            const x = entrance.x + dx, y = entrance.y + dy;
            const k = `${x},${y}`;
            if (waitSeen.has(k)) continue;
            if (mapRenderer.isWalkable(x, y)) {
              waitSeen.add(k);
              waitTiles.push({ x, y });
            }
          }
        }
      }
      if (waitTiles.length === 0) waitTiles.push(entrance);
      const GOD_SEAT = 0;
      const claimSeat = (agent) => {
        if (agent.isGod) {
          seatClaims.add(GOD_SEAT);
          return GOD_SEAT;
        }
        for (let i = 1; i < seatTiles.length; i++) {
          if (!seatClaims.has(i)) {
            seatClaims.add(i);
            return i;
          }
        }
        return null;
      };
      const facingForSeat = (t2) => {
        if (!mapRenderer.isWalkable(t2.x, t2.y - 1)) return "up";
        if (!mapRenderer.isWalkable(t2.x, t2.y + 1)) return "down";
        if (!mapRenderer.isWalkable(t2.x - 1, t2.y)) return "left";
        if (!mapRenderer.isWalkable(t2.x + 1, t2.y)) return "right";
        return "up";
      };
      const cafeSpots = [];
      const faceFurniture = (t2) => {
        if (!mapRenderer.isWalkable(t2.x + 1, t2.y)) return "right";
        if (!mapRenderer.isWalkable(t2.x - 1, t2.y)) return "left";
        if (!mapRenderer.isWalkable(t2.x, t2.y - 1)) return "up";
        return "down";
      };
      for (const name of theme.cafeSeatNames) {
        const p = mapRenderer.getSpawnPoint(name);
        if (p) cafeSpots.push({ tile: p, facing: facingForSeat(p), spot: "table", seated: true, partner: -1 });
      }
      for (let i = 0; i < cafeSpots.length; i++) {
        for (let j = i + 1; j < cafeSpots.length; j++) {
          const a = cafeSpots[i].tile, b = cafeSpots[j].tile;
          if (a.x === b.x && Math.abs(a.y - b.y) === 2) {
            cafeSpots[i].partner = j;
            cafeSpots[j].partner = i;
          }
        }
      }
      for (const [name, spot] of theme.cafeStands) {
        const p = mapRenderer.getSpawnPoint(name);
        if (p) cafeSpots.push({ tile: p, facing: faceFurniture(p), spot, seated: false, partner: -1 });
      }
      const cafeTaken = new Array(cafeSpots.length).fill(null);
      const agentById = (id) => useStore.getState().agents.find((a) => a.id === id);
      const TRAY_TILE = theme.coffee.trayTile;
      const TRAY_STAND = theme.coffee.trayStand;
      const MACHINE_STAND = theme.coffee.machineStand;
      const SINK_TILE = theme.coffee.sinkTile;
      const SINK_STAND = theme.coffee.sinkStand;
      const MAX_CUPS = theme.coffee.maxCups;
      let cleanCups = MAX_CUPS;
      const ts0 = mapRenderer.tileSize;
      const trayG = new Graphics();
      trayG.eventMode = "none";
      trayG.position.set(TRAY_TILE.x * ts0, TRAY_TILE.y * ts0);
      trayG.zIndex = (TRAY_TILE.y + 1) * ts0;
      charLayer.addChild(trayG);
      const drawTray = () => {
        trayG.clear();
        const slots = [[2, 10], [9, 10], [2, 15], [9, 15]];
        for (let i = 0; i < cleanCups && i < slots.length; i++) {
          paintCup(trayG, slots[i][0], slots[i][1]);
        }
      };
      drawTray();
      const sinkG = new Graphics();
      sinkG.eventMode = "none";
      sinkG.position.set(SINK_TILE.x * ts0, SINK_TILE.y * ts0);
      sinkG.zIndex = (SINK_TILE.y + 1) * ts0;
      charLayer.addChild(sinkG);
      let sinkBusy = 0;
      const drawSink = (t2) => {
        sinkG.clear();
        sinkG.rect(2, 6, 12, 8).fill(12174025);
        sinkG.rect(3, 7, 10, 6).fill(8885149);
        sinkG.rect(7, 9, 2, 2).fill(6121327);
        sinkG.rect(7, 2, 2, 4).fill(7042688);
        sinkG.rect(6, 2, 4, 1).fill(7042688);
        if (sinkBusy > 0) {
          sinkG.rect(7, 6, 2, 4).fill({ color: 10475248, alpha: 0.9 });
          for (let i = 0; i < 3; i++) {
            const ph = (t2 * 1.2 + i / 3) % 1;
            sinkG.circle(4 + i * 4, 7 - ph * 4, 1).fill({ color: 16777215, alpha: 0.7 * (1 - ph) });
          }
        }
      };
      drawSink(0);
      const machineG = new Graphics();
      machineG.eventMode = "none";
      machineG.position.set(26 * ts0, 17 * ts0);
      machineG.zIndex = 19 * ts0;
      charLayer.addChild(machineG);
      let machineBusy = 0;
      const drawMachine = (t2) => {
        machineG.clear();
        if (machineBusy <= 0) return;
        for (let i = 0; i < 2; i++) {
          const ph = (t2 * 0.9 + i * 0.5) % 1;
          machineG.rect(6 + i * 3, 2 - Math.round(ph * 5), 1, 1).fill({ color: 16777215, alpha: 0.6 * (1 - ph) });
        }
      };
      const finishRun = (rt) => {
        rt.run = void 0;
        const c = rt.character;
        if (c.isCarryingCup()) {
          rt.cupCarryHome = true;
          c.hideThought();
          c.sitAtDesk(false);
        } else {
          c.hideThought();
          c.startWandering();
        }
      };
      const startRunLeg = (rt, phase) => {
        rt.run = { phase, timer: 0 };
        const c = rt.character;
        const dest = phase === "toMachine" ? MACHINE_STAND : phase === "toSink" ? SINK_STAND : TRAY_STAND;
        c.walkToAndThen(dest, () => {
          if (!rt.run || rt.run.phase !== phase) return;
          c.faceDirection("up");
          if (phase === "toTray") {
            if (cleanCups <= 0) {
              c.showThought(t("office.mugs.empty"));
              rt.run = { phase: "placing", timer: -1 };
              return;
            }
            cleanCups--;
            drawTray();
            c.setCarryingCup(true);
            rt.run = { phase: "taking", timer: 0 };
          } else if (phase === "toMachine") {
            c.showThought(t("office.mugs.brewing"));
            machineBusy = 2.6;
            rt.run = { phase: "brewing", timer: 0 };
          } else if (phase === "toSink") {
            c.showThought(t("office.mugs.washing"));
            sinkBusy = 2.4;
            rt.run = { phase: "washing", timer: 0 };
          } else {
            c.setCarryingCup(false);
            cleanCups = Math.min(MAX_CUPS, cleanCups + 1);
            drawTray();
            rt.run = { phase: "placing", timer: 0 };
          }
        });
      };
      const releaseRun = (rt) => {
        if (!rt.run) return;
        rt.run = void 0;
        if (rt.character.isCarryingCup()) rt.cupCarryHome = true;
      };
      let fxClock = 0;
      const updateCoffeeRuns = (dt) => {
        fxClock += dt;
        if (sinkBusy > 0) {
          sinkBusy -= dt;
          drawSink(fxClock);
        }
        if (machineBusy > 0) {
          machineBusy -= dt;
          drawMachine(fxClock);
        }
        for (const [, rt] of runtimes) {
          const run = rt.run;
          if (!run) continue;
          run.timer += dt;
          rt.character;
          switch (run.phase) {
            case "toTray":
            case "toMachine":
            case "toSink":
            case "toTrayBack":
              if (run.timer > 20) finishRun(rt);
              break;
            case "taking":
              if (run.timer >= 0.8) startRunLeg(rt, "toMachine");
              break;
            case "brewing":
              if (run.timer >= 2.6) finishRun(rt);
              break;
            case "washing":
              if (run.timer >= 2.4) startRunLeg(rt, "toTrayBack");
              break;
            case "placing":
              if (run.timer >= 0.6) finishRun(rt);
              break;
          }
        }
      };
      const godDistance = (px, py) => {
        const god = useStore.getState().agents.find((a) => a.isGod);
        const grt = god ? runtimes.get(god.id) : void 0;
        if (!grt) return Infinity;
        const p = grt.character.getPixelPosition();
        return Math.hypot(p.x - px, p.y - py);
      };
      const emitQuip = (id, rt, spotIdx) => {
        const spot = cafeSpots[spotIdx];
        const character = agentById(id)?.character ?? DEFAULT_CHARACTER;
        const seed = Math.floor(Math.random() * 1e6);
        const p = rt.character.getPixelPosition();
        if (godDistance(p.x, p.y) > 96 && Math.random() < 0.35) {
          rt.character.showThought(t(GOSSIP_KEYS[Math.floor(Math.random() * GOSSIP_KEYS.length)]));
          return;
        }
        rt.character.showThought(pickSoloLine(character, spot.spot, seed));
      };
      const maybePairChat = (id, rt, spotIdx) => {
        const spot = cafeSpots[spotIdx];
        if (spot.partner < 0 || !rt.brk) return false;
        const partnerId = cafeTaken[spot.partner];
        if (!partnerId) return false;
        const prt = runtimes.get(partnerId);
        if (!prt?.brk || prt.brk.phase !== "lingering") return false;
        if (rt.brk.chat || rt.brk.chattingWith || prt.brk.chat || prt.brk.chattingWith) return false;
        const character = agentById(id)?.character ?? DEFAULT_CHARACTER;
        const lines = pickExchange(character, Math.floor(Math.random() * 1e6));
        rt.brk.chat = { lines, partnerId, idx: 0, beat: 0 };
        prt.brk.chattingWith = id;
        return true;
      };
      const releaseBreak = (rt) => {
        if (!rt.brk) return;
        if (rt.brk.chat) {
          const p = runtimes.get(rt.brk.chat.partnerId);
          if (p?.brk) p.brk.chattingWith = void 0;
        }
        if (rt.brk.chattingWith) {
          const o = runtimes.get(rt.brk.chattingWith);
          if (o?.brk) o.brk.chat = void 0;
        }
        cafeTaken[rt.brk.spotIdx] = null;
        rt.brk = void 0;
      };
      const endBreak = (id, rt) => {
        const arrived = rt.brk?.phase === "lingering";
        releaseBreak(rt);
        rt.character.hideThought();
        const agent = agentById(id);
        if (agent?.isGod) {
          rt.character.sitAtDesk(true);
          return;
        }
        const c = rt.character;
        if (!arrived) {
          if (c.isCarryingCup()) {
            rt.cupCarryHome = true;
            c.sitAtDesk(false);
          } else c.startWandering();
          return;
        }
        if (c.isCarryingCup()) {
          if (Math.random() < 0.6) startRunLeg(rt, "toMachine");
          else startRunLeg(rt, "toSink");
        } else if (!c.hasCupOnDesk() && Math.random() < 0.75) {
          startRunLeg(rt, "toTray");
        } else {
          c.startWandering();
        }
      };
      const startBreak = (id, rt) => {
        const free = [];
        const social = [];
        for (let i = 0; i < cafeSpots.length; i++) {
          if (cafeTaken[i]) continue;
          free.push(i);
          const p = cafeSpots[i].partner;
          if (p >= 0 && cafeTaken[p]) social.push(i);
        }
        if (free.length === 0) return;
        const pool = social.length && Math.random() < 0.55 ? social : free;
        const idx = pool[Math.floor(Math.random() * pool.length)];
        const spot = cafeSpots[idx];
        cafeTaken[idx] = id;
        rt.brk = { spotIdx: idx, phase: "walking", timer: 0, quipTimer: 0 };
        const c = rt.character;
        if (c.hasCupOnDesk()) {
          c.setCupOnDesk(false);
          c.setCarryingCup(true);
        }
        c.walkToAndThen(spot.tile, () => {
          if (!rt.brk || rt.brk.spotIdx !== idx) return;
          if (spot.seated) c.sitInPlace(spot.facing);
          else {
            c.setIdle();
            c.faceDirection(spot.facing);
          }
          rt.brk.phase = "lingering";
          rt.brk.timer = 8 + Math.random() * 8;
          rt.brk.quipTimer = 4 + Math.random() * 4;
          if (!maybePairChat(id, rt, idx)) emitQuip(id, rt, idx);
        });
      };
      const breakEligible = (agent, rt) => {
        if (agent.isGod || rt.brk || rt.err || rt.run || rt.cupCarryHome) return false;
        if (agent.status !== "idle" && agent.status !== "success") return false;
        return !rt.character.isSitting();
      };
      let cafeCooldown = 5;
      const updateCafeteria = (dt) => {
        for (const [id, rt2] of runtimes) {
          const b = rt2.brk;
          if (!b) continue;
          if (b.phase === "walking") {
            b.timer += dt;
            if (b.timer > 20) endBreak(id, rt2);
            continue;
          }
          if (b.chat) {
            b.chat.beat -= dt;
            if (b.chat.beat <= 0) {
              if (b.chat.idx < b.chat.lines.length) {
                const speaker = b.chat.idx % 2 === 0 ? rt2 : runtimes.get(b.chat.partnerId);
                speaker?.character.showThought(b.chat.lines[b.chat.idx]);
                b.chat.idx++;
                b.chat.beat = 2.4;
                b.timer = Math.max(b.timer, 3.5);
                const prt = runtimes.get(b.chat.partnerId);
                if (prt?.brk) prt.brk.timer = Math.max(prt.brk.timer, 3.5);
              } else {
                const prt = runtimes.get(b.chat.partnerId);
                if (prt?.brk) prt.brk.chattingWith = void 0;
                b.chat = void 0;
              }
            }
          } else if (!b.chattingWith) {
            b.quipTimer -= dt;
            if (b.quipTimer <= 0) {
              b.quipTimer = 4 + Math.random() * 4;
              emitQuip(id, rt2, b.spotIdx);
            } else if (Math.random() < 4e-3) maybePairChat(id, rt2, b.spotIdx);
          }
          b.timer -= dt;
          if (b.timer <= 0) endBreak(id, rt2);
        }
        cafeCooldown -= dt;
        if (cafeCooldown > 0) return;
        cafeCooldown = 6 + Math.random() * 6;
        if (cafeTaken.filter(Boolean).length >= 4) return;
        if (Math.random() >= 0.7) return;
        const candidates = [];
        for (const agent2 of useStore.getState().agents) {
          const rt2 = runtimes.get(agent2.id);
          if (rt2 && breakEligible(agent2, rt2)) candidates.push([agent2, rt2]);
        }
        if (candidates.length === 0) return;
        const [agent, rt] = candidates[Math.floor(Math.random() * candidates.length)];
        startBreak(agent.id, rt);
      };
      const ERRAND_SPOTS = theme.errandSpots;
      const errandTaken = new Array(ERRAND_SPOTS.length).fill(null);
      const errandFx = /* @__PURE__ */ new Map();
      const fxFor = (idx) => {
        let g = errandFx.get(idx);
        if (!g) {
          const spot = ERRAND_SPOTS[idx];
          g = new Graphics();
          g.eventMode = "none";
          g.position.set(spot.fx.x * ts0, spot.fx.y * ts0);
          g.zIndex = (spot.fx.y + 1) * ts0;
          charLayer.addChild(g);
          errandFx.set(idx, g);
        }
        return g;
      };
      const drawErrandFx = (kind, g, t2) => {
        g.clear();
        if (kind === "window" || kind === "smoke") {
          for (let i = 0; i < 3; i++) {
            const ph = (t2 * 0.7 + i / 3) % 1;
            g.rect(2 + i * 9 - ph * 5, 26 + ph * 16, 7, 1).fill({ color: 14217719, alpha: 0.55 * (1 - ph) });
          }
        } else if (kind === "dispenser") {
          const ph = t2 * 1.6 % 1;
          g.rect(7, 18 + ph * 6, 1, 3).fill({ color: 10475248, alpha: 0.9 * (1 - ph) });
          const bp = t2 * 0.9 % 1;
          g.circle(8, 12 - bp * 6, 1).fill({ color: 16777215, alpha: 0.6 * (1 - bp) });
        } else if (kind === "fridge") {
          const a = 0.16 + 0.05 * Math.sin(t2 * 5);
          g.poly([3, 12, 13, 12, 16, 30, 0, 30]).fill({ color: 16773816, alpha: a });
        } else if (kind === "shelf") {
          const ph = t2 * 0.5 % 1;
          g.rect(2 + ph * 24, 4 + Math.floor(t2 * 0.5) % 3 * 9, 2, 2).fill({ color: 16775112, alpha: 0.8 * Math.sin(ph * Math.PI) });
        } else if (kind === "bin") {
          const ph = t2 * 1 % 1;
          if (ph < 0.45) {
            const p = ph / 0.45;
            const fromX = 18, toX = 8;
            const x = fromX + (toX - fromX) * p;
            const y = 2 - Math.sin(p * Math.PI) * 9;
            g.rect(Math.round(x), Math.round(y), 2, 2).fill({ color: 16118246, alpha: 0.95 });
          }
        }
      };
      const releaseErrand = (rt) => {
        if (!rt.err) return;
        errandTaken[rt.err.idx] = null;
        errandFx.get(rt.err.idx)?.clear();
        rt.err = void 0;
        rt.character.stopWatering();
        rt.character.stopSmoking();
      };
      let errCooldown = 18;
      const updateErrands = (dt) => {
        for (const [, rt2] of runtimes) {
          const err = rt2.err;
          if (!err) continue;
          err.timer += dt;
          const spot2 = ERRAND_SPOTS[err.idx];
          if (err.phase === "walking") {
            if (err.timer > 20) {
              releaseErrand(rt2);
              rt2.character.startWandering();
            }
            continue;
          }
          drawErrandFx(spot2.kind, fxFor(err.idx), err.timer);
          if (spot2.kind !== "water" && spot2.kind !== "smoke" && err.timer >= spot2.duration) {
            releaseErrand(rt2);
            rt2.character.hideThought();
            rt2.character.startWandering();
          }
        }
        errCooldown -= dt;
        if (errCooldown > 0) return;
        errCooldown = 14 + Math.random() * 18;
        if (Math.random() >= 0.65) return;
        const free = ERRAND_SPOTS.map((_, i) => i).filter((i) => !errandTaken[i]);
        if (free.length === 0) return;
        const idx = free[Math.floor(Math.random() * free.length)];
        const spot = ERRAND_SPOTS[idx];
        let agent;
        let rt;
        if (spot.godOnly) {
          return;
        } else {
          const candidates = [];
          for (const a of useStore.getState().agents) {
            const r = runtimes.get(a.id);
            if (r && breakEligible(a, r)) candidates.push([a, r]);
          }
          if (candidates.length === 0) return;
          [agent, rt] = candidates[Math.floor(Math.random() * candidates.length)];
        }
        const c = rt.character;
        errandTaken[idx] = agent.id;
        rt.err = { phase: "walking", timer: 0, idx };
        c.walkToAndThen(spot.stand, () => {
          if (!rt.err || rt.err.idx !== idx) return;
          rt.err.phase = "doing";
          rt.err.timer = 0;
          c.faceDirection(spot.facing);
          const lines = ERRAND_THOUGHTS[spot.kind];
          c.showThought(t(lines[Math.floor(Math.random() * lines.length)]));
          const finish = () => {
            const wasGod = !!agent.isGod;
            releaseErrand(rt);
            c.hideThought();
            if (wasGod) c.sitAtDesk(true);
            else c.startWandering();
          };
          if (spot.kind === "water") c.startWatering(spot.duration, finish);
          else if (spot.kind === "smoke") c.startSmoking(spot.duration, finish);
        });
      };
      const lastSuckUp = /* @__PURE__ */ new Map();
      let doneByAssignee = /* @__PURE__ */ new Map();
      let statsAge = 999;
      let auraCooldown = 1.5;
      const updateBossAura = (dt) => {
        statsAge += dt;
        if (statsAge > 30) {
          statsAge = 0;
          void window.cth.hiveTasks().then((raw) => {
            const arr = raw && typeof raw === "object" && Array.isArray(raw.tasks) ? raw.tasks : [];
            const m = /* @__PURE__ */ new Map();
            for (const t2 of arr) {
              if (t2?.status === "done" && typeof t2.assignee === "string" && t2.assignee) {
                m.set(t2.assignee, (m.get(t2.assignee) ?? 0) + 1);
              }
            }
            doneByAssignee = m;
          }).catch(() => {
          });
        }
        auraCooldown -= dt;
        if (auraCooldown > 0) return;
        auraCooldown = 1.5;
        const god = useStore.getState().agents.find((a) => a.isGod);
        const grt = god ? runtimes.get(god.id) : void 0;
        if (!grt) return;
        const gp = grt.character.getPixelPosition();
        const now = Date.now();
        for (const [id, rt] of runtimes) {
          if (id === god.id) continue;
          const a = agentById(id);
          if (!a) continue;
          if (a.status !== "idle" && a.status !== "success") continue;
          if (rt.brk?.chat || rt.brk?.chattingWith) continue;
          const p = rt.character.getPixelPosition();
          if (Math.hypot(p.x - gp.x, p.y - gp.y) > 44) continue;
          if (now - (lastSuckUp.get(id) ?? 0) < 25e3) continue;
          if (Math.random() >= 0.6) continue;
          lastSuckUp.set(id, now);
          const done = doneByAssignee.get(id) ?? 0;
          const pool = done > 0 ? SUCK_UP_KEYS : SUCK_UP_KEYS.slice(2);
          const line = pool[Math.floor(Math.random() * pool.length)];
          rt.character.showThought(t(line, { done: String(done) }));
          rt.character.hideThought();
        }
      };
      const updateDeskLife = (dt) => {
        for (const [id, rt] of runtimes) {
          if (rt.cupCarryHome && rt.character.isSittingAtDesk()) {
            rt.cupCarryHome = false;
            rt.character.setCarryingCup(false);
            rt.character.setCupOnDesk(true);
            const agent = agentById(id);
            if (agent && !agent.isGod && (agent.status === "idle" || agent.status === "success")) {
              rt.character.startWandering();
            }
          }
          if (rt.screen) {
            rt.screen.setOn(rt.character.isSittingAtDesk());
            rt.screen.update(dt);
          }
        }
      };
      const BOARD_TILE = theme.anchors.boards;
      const BOARD_CENTER_PAD = 15;
      const NOTE_COLORS = theme.palette.noteColors;
      const tsB = mapRenderer.tileSize;
      const boardG = new Graphics();
      boardG.eventMode = "static";
      boardG.cursor = "pointer";
      boardG.position.set(BOARD_TILE.x * tsB + BOARD_CENTER_PAD, BOARD_TILE.y * tsB);
      boardG.zIndex = (BOARD_TILE.y + 1) * tsB;
      boardG.on("pointertap", (ev) => {
        ev.stopPropagation();
        const st = useStore.getState();
        const god = st.agents.find((a) => a.isGod);
        if (god) st.select(god.id);
        st.requestCommandCenterTab("tasks");
      });
      charLayer.addChild(boardG);
      const deskNoteG = /* @__PURE__ */ new Map();
      const clearDeskNotes = () => {
        for (const g of deskNoteG.values()) {
          g.parent?.removeChild(g);
          g.destroy();
        }
        deskNoteG.clear();
      };
      const drawCork = (ox, header, notes) => {
        boardG.rect(ox, -8, 30, 22).fill(7231033);
        boardG.rect(ox + 1, -7, 28, 3).fill(header);
        boardG.rect(ox + 1, -4, 28, 17).fill(13217923);
        const n = Math.min(notes.length, 12);
        for (let i = 0; i < n; i++) {
          const x = ox + 3 + i % 4 * 7;
          const y = -2 + Math.floor(i / 4) * 5;
          boardG.rect(x, y, 5, 4).fill(NOTE_COLORS[notes[i]] ?? 15920604);
          boardG.rect(x + 2, y, 1, 1).fill(4864850);
        }
        if (notes.length > 12) {
          boardG.rect(ox + 22, 8, 5, 4).fill(15261896);
          boardG.rect(ox + 23, 7, 5, 4).fill(15920604);
        }
      };
      const drawTaskBoard = (tasks) => {
        boardG.clear();
        clearDeskNotes();
        const blocked = tasks.filter((t2) => t2.status === "blocked").map(() => "blocked");
        const todoNotes = tasks.filter((t2) => t2.status === "todo").map(() => "todo");
        let done = 0;
        for (const t2 of tasks) {
          if (t2.status === "done") {
            done++;
            continue;
          }
          if (t2.status !== "doing") continue;
          const rt = t2.assignee ? runtimes.get(t2.assignee) : void 0;
          if (!rt) {
            todoNotes.push("doing");
            continue;
          }
          const desk = rt.character.getDeskTile();
          let g = deskNoteG.get(t2.assignee);
          if (!g) {
            g = new Graphics();
            g.eventMode = "none";
            g.position.set((desk.x - 1) * tsB + 3, (desk.y - 1) * tsB + 8);
            g.zIndex = desk.y * tsB - 1;
            charLayer.addChild(g);
            deskNoteG.set(t2.assignee, g);
          }
          const idx = g.__count ?? 0;
          g.__count = idx + 1;
          g.rect(idx * 7, -(idx % 2), 5, 4).fill(NOTE_COLORS.doing);
          g.rect(idx * 7 + 2, -(idx % 2), 1, 1).fill(4864850);
        }
        drawCork(0, NOTE_COLORS.blocked, blocked);
        drawCork(34, NOTE_COLORS.todo, todoNotes);
        boardG.rect(68, 6, 14, 4).fill(11570526);
        boardG.rect(68, 10, 14, 4).fill(9072461);
        boardG.rect(69, 14, 2, 2).fill(7231033);
        boardG.rect(79, 14, 2, 2).fill(7231033);
        const stack = Math.min(done, 6);
        for (let i = 0; i < stack; i++) {
          boardG.rect(71 + i % 2, 4 - i * 2, 8, 2).fill({ color: NOTE_COLORS.done, alpha: 1 }).stroke({ color: 7245678, width: 0.5 });
        }
      };
      drawTaskBoard([]);
      const clockG = new Graphics();
      clockG.eventMode = "static";
      clockG.cursor = "pointer";
      clockG.position.set(theme.anchors.clock.x * ts0, theme.anchors.clock.y * ts0);
      clockG.hitArea = { contains: (x, y) => x >= 0 && x <= 16 && y >= 0 && y <= 32 };
      clockG.zIndex = 3 * ts0;
      clockG.on("pointertap", (ev) => {
        ev.stopPropagation();
        window.close();
      });
      charLayer.addChild(clockG);
      const askG = new Graphics();
      askG.eventMode = "static";
      askG.cursor = "pointer";
      askG.position.set(14 * tsB + 25, 10 * tsB);
      askG.zIndex = 11 * tsB;
      askG.on("pointertap", (ev) => {
        ev.stopPropagation();
        const st = useStore.getState();
        const god = st.agents.find((a) => a.isGod);
        if (god) st.select(god.id);
        st.requestCommandCenterTab("human");
      });
      charLayer.addChild(askG);
      let askCount = 0;
      let askPulse = 0;
      const drawAskBoard = (pulse) => {
        askG.clear();
        askG.rect(0, -8, 30, 22).fill(5982827);
        askG.rect(1, -7, 28, 3).fill(13481192);
        askG.rect(1, -4, 28, 17).fill(13217923);
        if (askCount === 0) {
          askG.rect(13, -1, 4, 2).fill({ color: 9074015, alpha: 0.8 });
          askG.rect(15, 1, 2, 4).fill({ color: 9074015, alpha: 0.8 });
          askG.rect(15, 7, 2, 2).fill({ color: 9074015, alpha: 0.8 });
        } else {
          const n = Math.min(askCount, 8);
          for (let i = 0; i < n; i++) {
            const x = 3 + i % 4 * 7;
            const y = -2 + Math.floor(i / 4) * 6;
            askG.rect(x, y, 5, 4).fill(13481192);
            askG.rect(x + 2, y, 1, 1).fill(4864850);
          }
          const a = 0.35 + 0.3 * Math.sin(pulse * 4);
          askG.rect(-2, -10, 34, 26).stroke({ color: 13481192, width: 2, alpha: a });
        }
      };
      drawAskBoard(0);
      const PIN_STAND = { x: 8, y: 11 };
      const TAKE_STAND = { x: 9, y: 11 };
      const ARCHIVE_STAND = { x: 12, y: 11 };
      let visualTasks = /* @__PURE__ */ new Map();
      const moveQueue = [];
      const busyActors = /* @__PURE__ */ new Set();
      const carriedNotes = /* @__PURE__ */ new Map();
      const redrawVisual = () => drawTaskBoard([...visualTasks.values()]);
      const finishMove = (mv, rt) => {
        visualTasks.set(mv.taskId, mv.after);
        redrawVisual();
        busyActors.delete(mv.actorId);
        const g = carriedNotes.get(mv.actorId);
        if (g) {
          g.parent?.removeChild(g);
          g.destroy();
          carriedNotes.delete(mv.actorId);
        }
        if (rt) {
          rt.character.hideThought();
          const agent = agentById(mv.actorId);
          if (agent) applyState(agent, rt, true);
        }
      };
      const attachCarriedNote = (actorId, color) => {
        if (carriedNotes.has(actorId)) return;
        const g = new Graphics();
        g.eventMode = "none";
        g.rect(0, 0, 5, 4).fill(color);
        g.rect(2, 0, 1, 1).fill(4864850);
        charLayer.addChild(g);
        carriedNotes.set(actorId, g);
      };
      const startMove = (mv) => {
        const rt = runtimes.get(mv.actorId);
        if (!rt) {
          finishMove(mv, void 0);
          return;
        }
        busyActors.add(mv.actorId);
        const c = rt.character;
        if (mv.kind === "archive") {
          attachCarriedNote(mv.actorId, mv.carryColor);
          visualTasks.set(mv.taskId, { status: "__carried__" });
          redrawVisual();
        }
        c.showThought(mv.thought);
        c.walkToAndThen(mv.stand, () => {
          c.faceDirection("up");
          if (mv.kind === "take") attachCarriedNote(mv.actorId, mv.carryColor);
          setTimeout(() => {
            if (mv.kind === "take") {
              const rt2 = runtimes.get(mv.actorId);
              if (!rt2) {
                finishMove(mv, void 0);
                return;
              }
              visualTasks.set(mv.taskId, { ...mv.after, status: "__carried__" });
              redrawVisual();
              rt2.character.walkToAndThen(rt2.character.getDeskTile(), () => finishMove(mv, rt2));
            } else {
              finishMove(mv, runtimes.get(mv.actorId));
            }
          }, 900);
        });
      };
      let moveWatchdog = 0;
      const updateBoardMoves = (dt) => {
        for (const [id, g] of carriedNotes) {
          const rt = runtimes.get(id);
          if (!rt) continue;
          const p = rt.character.getPixelPosition();
          g.position.set(p.x + 5, p.y - 10);
          g.zIndex = p.y + 1;
        }
        for (let i = moveQueue.length - 1; i >= 0; i--) {
          if (!busyActors.has(moveQueue[i].actorId)) {
            const mv = moveQueue.splice(i, 1)[0];
            startMove(mv);
          }
        }
        askPulse += dt;
        if (askCount > 0) drawAskBoard(askPulse);
        moveWatchdog += dt;
        if (moveWatchdog > 30 && busyActors.size > 0) {
          moveWatchdog = 0;
          for (const id of [...busyActors]) {
            busyActors.delete(id);
            const g = carriedNotes.get(id);
            if (g) {
              g.parent?.removeChild(g);
              g.destroy();
              carriedNotes.delete(id);
            }
          }
          visualTasks = new Map(lastLedger.map((t2) => [t2.id, { status: t2.status, assignee: t2.assignee }]));
          redrawVisual();
        }
      };
      const actorFor = (assignee, preferGod) => {
        const agent = useStore.getState().agents.find((a) => a.id === assignee);
        if (!preferGod && assignee && agent && !agent.isGod && runtimes.has(assignee)) return assignee;
        return void 0;
      };
      let lastLedger = [];
      let firstPoll = true;
      const pollTaskBoard = async () => {
        try {
          const raw = await window.cth.hiveTasks();
          const arr = raw && Array.isArray(raw.tasks) ? raw.tasks : [];
          const ledger = arr.map((t2, i) => ({
            id: typeof t2?.id === "string" && t2.id ? t2.id : `idx-${i}`,
            status: String(t2?.status ?? "todo"),
            assignee: typeof t2?.assignee === "string" && t2.assignee ? t2.assignee : void 0
          }));
          const newAsk = arr.filter((t2) => waitsOnHuman(t2)).length;
          if (newAsk !== askCount) {
            askCount = newAsk;
            drawAskBoard(askPulse);
          }
          if (firstPoll) {
            firstPoll = false;
            visualTasks = new Map(ledger.map((t2) => [t2.id, { status: t2.status, assignee: t2.assignee }]));
            redrawVisual();
            lastLedger = ledger;
            return;
          }
          const prev = new Map(lastLedger.map((t2) => [t2.id, t2]));
          let instant = false;
          for (const t2 of ledger) {
            const old = prev.get(t2.id);
            const oldS = old?.status;
            if (oldS === t2.status && old?.assignee === t2.assignee) continue;
            const after = { status: t2.status, assignee: t2.assignee };
            let mv = null;
            if (!old && (t2.status === "todo" || t2.status === "blocked")) {
              const actor = actorFor(void 0, true);
              if (actor) mv = { kind: "pin", taskId: t2.id, actorId: actor, after, carryColor: NOTE_COLORS[t2.status], stand: t2.status === "blocked" ? PIN_STAND : TAKE_STAND, thought: "pinning a new task 📌" };
            } else if (oldS !== "doing" && t2.status === "doing") {
              const actor = actorFor(t2.assignee, false);
              if (actor && actor === t2.assignee) mv = { kind: "take", taskId: t2.id, actorId: actor, after, carryColor: NOTE_COLORS.doing, stand: TAKE_STAND, thought: "grabbing my task" };
            } else if (t2.status === "done" && oldS !== "done") {
              const actor = actorFor(old?.assignee ?? t2.assignee, false);
              if (actor) mv = { kind: "archive", taskId: t2.id, actorId: actor, after, carryColor: NOTE_COLORS.done, stand: ARCHIVE_STAND, thought: "filing it as done ✔" };
            } else if (t2.status === "blocked" && oldS !== "blocked") {
              const actor = actorFor(old?.assignee ?? t2.assignee, false);
              if (actor) mv = { kind: "pin", taskId: t2.id, actorId: actor, after, carryColor: NOTE_COLORS.blocked, stand: PIN_STAND, thought: "this one is stuck 😤" };
            }
            if (mv && !busyActors.has(mv.actorId) && !moveQueue.some((q) => q.actorId === mv.actorId)) {
              if (!visualTasks.has(t2.id) && mv.kind !== "pin") visualTasks.set(t2.id, { status: oldS ?? "todo", assignee: old?.assignee });
              moveQueue.push(mv);
            } else {
              visualTasks.set(t2.id, after);
              instant = true;
            }
          }
          for (const id of [...visualTasks.keys()]) {
            if (!ledger.some((t2) => t2.id === id)) {
              visualTasks.delete(id);
              instant = true;
            }
          }
          if (instant) redrawVisual();
          lastLedger = ledger;
        } catch {
        }
      };
      if (!pausedRef.current) void pollTaskBoard();
      const taskBoardPoll = setInterval(() => {
        if (!pausedRef.current) void pollTaskBoard();
      }, 5e3);
      app.__taskBoardPoll = taskBoardPoll;
      taskBoardPollRef.current = () => {
        if (mountIdRef.current === mountId) void pollTaskBoard();
      };
      app.__unsubAvatars = subscribeAvatars(() => {
        for (const [id, rt] of runtimes) {
          if (!isCustomAvatarId(rt.charName)) continue;
          void theme.cast.getFrames(rt.charName).then((frames) => {
            if (mountIdRef.current === mountId && runtimes.get(id) === rt) rt.character.setFrames(frames);
          });
        }
      });
      const addCharacter = async (agent) => {
        const custom = theme.cast.byName[agent.character] ? void 0 : castMemberFor(agent.character);
        const charName = theme.cast.byName[agent.character] || custom ? agent.character : theme.cast.defaultCharacter;
        const member = theme.cast.byName[charName] ?? custom ?? theme.cast.byName[theme.cast.defaultCharacter];
        const seatIndex = claimSeat(agent);
        const seatTile = (seatIndex != null ? seatTiles[seatIndex] : void 0) ?? mapRenderer.getSpawnPoint("entrance") ?? { x: 2, y: 2 };
        const waitTile = waitTiles[(seatIndex ?? 0) % waitTiles.length];
        let frames;
        try {
          frames = await theme.cast.getFrames(charName);
        } catch (err) {
          if (seatIndex != null) seatClaims.delete(seatIndex);
          throw err;
        }
        if (mountIdRef.current !== mountId) return;
        if (!useStore.getState().agents.some((a) => a.id === agent.id)) {
          if (seatIndex != null) seatClaims.delete(seatIndex);
          return;
        }
        const character = new Character({
          agentId: agent.id,
          displayName: agent.name,
          mapRenderer,
          frames,
          seatTile,
          seatDirection: facingForSeat(seatTile),
          spawnTile: agent.isGod ? seatTile : entrance,
          // captain starts at the reserved chair; others walk in
          glowColor: hexNum(colors.accent[agent.accent]) ?? hexToNumber(member.shirt),
          onClick: (id) => useStore.getState().select(id),
          noNameTag: onPage,
          bubbleArea: theme.art
        });
        character.show(charLayer);
        const rt = { character, seatIndex, waitTile, charName };
        if (mapRenderer.gidAt("furniture-above", seatTile.x, seatTile.y - 2) === theme.monitor.offTopLeftGid) {
          const top = { x: seatTile.x, y: seatTile.y - 2 };
          rt.screen = new DeskScreen(mapRenderer, top, theme.monitor);
          charLayer.addChild(rt.screen.container);
          const ts2 = mapRenderer.tileSize;
          character.setCupSpot({ x: top.x * ts2 + 18, y: top.y * ts2 + 23 });
        }
        runtimes.set(agent.id, rt);
        applyState(agent, rt, true);
      };
      const removeCharacter = (id) => {
        const rt = runtimes.get(id);
        if (!rt) return;
        releaseBreak(rt);
        releaseErrand(rt);
        releaseRun(rt);
        if (rt.character.isCarryingCup() || rt.character.hasCupOnDesk()) {
          if (cleanCups >= MAX_CUPS) console.warn("[office] mug reclaim over cap — cup accounting drifted");
          cleanCups = Math.min(MAX_CUPS, cleanCups + 1);
          drawTray();
        }
        if (rt.seatIndex != null) seatClaims.delete(rt.seatIndex);
        rt.screen?.destroy();
        rt.character.hide(0);
        setTimeout(() => rt.character.destroy(), 700);
        runtimes.delete(id);
      };
      let officeAsleep = false;
      const applyState = (agent, rt, force = false) => {
        const changed = force || rt.prevStatus !== agent.status || rt.prevAction !== agent.action || rt.prevCarrying !== agent.carrying || rt.prevPrompt !== agent.lastPrompt;
        if (!changed) return;
        const wasBusy = rt.prevStatus === "working" || rt.prevStatus === "thinking" || rt.prevStatus === "compacting";
        const isBusy = agent.status === "working" || agent.status === "thinking" || agent.status === "compacting";
        if (isBusy && !wasBusy) rt.busySince = Date.now();
        const finishedWork = !force && !agent.isGod && wasBusy && (agent.status === "idle" || agent.status === "success") && rt.busySince !== void 0 && Date.now() - rt.busySince >= CHEER_MIN_BUSY_MS;
        if (!isBusy) rt.busySince = void 0;
        rt.prevStatus = agent.status;
        rt.prevAction = agent.action;
        rt.prevCarrying = agent.carrying;
        rt.prevPrompt = agent.lastPrompt;
        const c = rt.character;
        c.setBaseAlpha(agent.status === "ghost" ? 0.5 : 1);
        if (agent.isGod) {
          if (rt.brk) releaseBreak(rt);
          if (rt.err) releaseErrand(rt);
          if (rt.run) releaseRun(rt);
          c.sitAtDesk(true);
          c.setStatusGlyph(agent.status === "blocked" ? "blocked" : "none");
          if (officeAsleep) c.hideThought();
          else c.showThought(activity(agent, t("office.activity.runningFloor")));
          return;
        }
        if (rt.brk) {
          if (agent.status === "idle" || agent.status === "success") {
            c.setStatusGlyph(agent.status === "success" ? "success" : "none");
            return;
          }
          releaseBreak(rt);
        }
        if (rt.err) {
          if (agent.status === "idle" || agent.status === "success") {
            c.setStatusGlyph(agent.status === "success" ? "success" : "none");
            return;
          }
          releaseErrand(rt);
        }
        if (rt.run) {
          if (agent.status === "idle" || agent.status === "success") {
            c.setStatusGlyph(agent.status === "success" ? "success" : "none");
            return;
          }
          releaseRun(rt);
        }
        switch (agent.status) {
          case "working":
          case "thinking":
            c.setStatusGlyph("none");
            c.sitAtDesk(true);
            c.showThought(activity(agent), agent.carrying);
            break;
          case "waiting":
            c.setStatusGlyph("none");
            c.sitAtDesk(false);
            c.showThought(activity(agent, t("office.activity.waiting")), agent.carrying);
            break;
          case "blocked":
            c.setStatusGlyph("blocked");
            c.showThought(activity(agent, t("office.activity.needsYou")));
            c.walkToTile(rt.waitTile);
            break;
          case "compacting":
            c.setStatusGlyph("compacting");
            c.sitAtDesk(true);
            c.showThought(activity(agent, t("office.activity.compacting")));
            break;
          case "looping":
            c.setStatusGlyph("looping");
            c.sitAtDesk(false);
            c.showThought(activity(agent, t("office.activity.looping")));
            break;
          case "success":
            c.setStatusGlyph("success");
            if (agent.isGod) {
              c.hideThought();
              c.sitAtDesk(true);
              break;
            }
            c.startWandering();
            if (finishedWork) {
              c.cheer();
              c.showThought(t(CHEER_KEYS[Math.floor(Math.random() * CHEER_KEYS.length)]));
            } else {
              c.hideThought();
            }
            break;
          case "ghost":
            c.setStatusGlyph("none");
            c.hideThought();
            c.setIdle();
            break;
          case "idle":
          default:
            c.setStatusGlyph("none");
            if (agent.isGod) {
              c.sitAtDesk(true);
              if (officeAsleep) c.hideThought();
              else c.showThought(activity(agent, t("office.activity.runningFloor")));
            } else if (finishedWork) {
              c.startWandering();
              c.cheer();
              c.showThought(t(CHEER_KEYS[Math.floor(Math.random() * CHEER_KEYS.length)]));
            } else {
              c.startWandering();
              if (onPage) c.hideThought();
              else c.showThought(activity(agent, t("office.activity.idle")));
            }
            break;
        }
      };
      const syncAgents = () => {
        const { agents } = useStore.getState();
        const present = new Set(agents.map((a) => a.id));
        for (const id of Array.from(runtimes.keys())) {
          if (!present.has(id)) removeCharacter(id);
        }
        for (const agent of agents) {
          const rt = runtimes.get(agent.id);
          if (!rt) {
            if (!pendingAdds.has(agent.id)) {
              pendingAdds.add(agent.id);
              void addCharacter(agent).catch((err) => console.warn("[office] addCharacter failed", err)).finally(() => {
                pendingAdds.delete(agent.id);
                if (mountIdRef.current === mountId) drawOnceWhilePaused(app.ticker, pausedRef.current);
              });
            }
          } else {
            rt.character.setName(agent.name);
            applyState(agent, rt);
          }
        }
      };
      syncAgents();
      const readSpot = (id) => {
        const c = runtimes.get(id)?.character;
        if (!c) return null;
        const b = c.getScreenBox(), at = offsetRef.current;
        return { ...b, x: b.x + at.x, y: b.y + at.y, clouded: !!c.getThoughtLayout() };
      };
      let lastSelected = useStore.getState().selectedId;
      const unsubscribe = useStore.subscribe((s, prev) => {
        if (s.agents !== prev.agents) syncAgents();
        if (s.selectedId !== lastSelected) {
          lastSelected = s.selectedId;
          const rt = s.selectedId ? runtimes.get(s.selectedId) : void 0;
          if (rt) {
            const p = rt.character.getPixelPosition();
            camera.nudgeToward(p.x, p.y);
          }
        }
      });
      app.__unsub = unsubscribe;
      app.__unsubAsleep = watchWindowOfficeAsleep((a) => {
        officeAsleep = a;
        for (const agent of useStore.getState().agents) {
          const rt = agent.isGod ? runtimes.get(agent.id) : void 0;
          if (rt) applyState(agent, rt, true);
        }
      });
      const ts = mapRenderer.tileSize;
      const humanPos = { x: entrance.x * ts + ts / 2, y: entrance.y * ts + ts };
      const posFor = (id) => {
        if (id === "human") return humanPos;
        const rt = runtimes.get(id);
        return rt ? rt.character.getPixelPosition() : null;
      };
      const spawnHandoff = (fromId, toId, act, needsHuman) => {
        if (envelopes.length >= MAX_ENVELOPES) return;
        if (toId === fromId) return;
        const from = posFor(fromId);
        const to = posFor(toId);
        if (!from || !to) return;
        const env = new MessageEnvelope(from, to, act, needsHuman);
        charLayer.addChild(env.container);
        envelopes.push(env);
      };
      const offMessage = window.cth.onHiveMessage ? window.cth.onHiveMessage((e) => {
        for (const target of e.targets) spawnHandoff(e.from, target, e.act, e.needsHuman);
      }) : () => {
      };
      const onDemoHandoff = (ev) => {
        const d = ev.detail;
        if (d) spawnHandoff(d.from, d.to, d.act, false);
      };
      window.addEventListener("cth:demo-handoff", onDemoHandoff);
      app.__offMessage = () => {
        offMessage();
        window.removeEventListener("cth:demo-handoff", onDemoHandoff);
      };
      const resolveBubbleOverlaps = () => {
        const items = [];
        for (const rt of runtimes.values()) {
          const lay = rt.character.getThoughtLayout();
          if (lay) items.push({ rt, ...lay });
        }
        if (items.length < 2) {
          for (const it of items) it.rt.character.setThoughtLift(0);
          return;
        }
        items.sort((a, b) => b.y + b.h - (a.y + a.h) || a.x - b.x);
        const placed = [];
        const pad = 2;
        for (const it of items) {
          let y = it.y;
          let moved = true, guard = 0;
          while (moved && guard++ < 12) {
            moved = false;
            for (const p of placed) {
              const overlapX = it.x < p.x + p.w + pad && it.x + it.w + pad > p.x;
              const overlapY = y < p.y + p.h + pad && y + it.h + pad > p.y;
              if (overlapX && overlapY) {
                y = p.y - it.h - pad;
                moved = true;
              }
            }
          }
          placed.push({ x: it.x, y, w: it.w, h: it.h });
          it.rt.character.setThoughtLift(it.y - y);
        }
      };
      let decor = null;
      const dressForSeason = () => {
        const on = currentSeason() === "halloween";
        if (on && !decor) {
          const tsD = mapRenderer.tileSize;
          const spawns = [...mapRenderer.getAllSpawnPoints().values()];
          const desks = [];
          for (const name of theme.primarySeatNames) {
            const seat = mapRenderer.getSpawnPoint(name);
            if (!seat) continue;
            if (mapRenderer.gidAt("furniture-above", seat.x, seat.y - 2) !== theme.monitor.offTopLeftGid) continue;
            desks.push({ x: seat.x, y: seat.y - 2 });
          }
          const hung = [calG, clockG, boardG, askG].map((g) => {
            const b = g.getLocalBounds();
            return { x0: g.x + b.minX, y0: g.y + b.minY, x1: g.x + b.maxX, y1: g.y + b.maxY };
          });
          const hungOver = (x, y) => hung.some((r) => r.x0 < (x + 1) * tsD + 4 && r.x1 > x * tsD - 4 && r.y0 < (y + 1) * tsD && r.y1 > y * tsD);
          decor = new SeasonDecor({
            plan: planHalloween({
              width: mapRenderer.width,
              height: mapRenderer.height,
              tileSize: tsD,
              wallAt: (x, y) => mapRenderer.gidAt("walls", x, y),
              isWalkable: (x, y) => mapRenderer.isWalkable(x, y),
              desks,
              machineStand: theme.coffee.machineStand,
              sinkTile: theme.coffee.sinkTile,
              decorAt: (x, y) => mapRenderer.gidAt("furniture-below", x, y) !== 0 || mapRenderer.gidAt("furniture-above", x, y) !== 0 || hungOver(x, y),
              avoid: [
                ...spawns,
                ...theme.errandSpots.map((e) => e.stand),
                theme.coffee.trayStand,
                theme.coffee.machineStand,
                theme.coffee.sinkStand
              ]
            }),
            mapWidth: mapRenderer.width * tsD,
            ...onPage ? { tintInset: 4 } : {},
            mapHeight: mapRenderer.height * tsD,
            charLayer,
            root: mapRenderer.getContainer(),
            reducedMotion: typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches
          });
        }
        decor?.setOn(on);
      };
      dressForSeason();
      const redressCast = () => {
        for (const [id, rt] of runtimes) {
          void theme.cast.getFrames(rt.charName).then((frames) => {
            if (mountIdRef.current === mountId && runtimes.get(id) === rt) rt.character.setFrames(frames);
          });
        }
      };
      app.__unsubSeason = subscribeSeason(() => {
        dressForSeason();
        redressCast();
      });
      const onTick = (ticker) => {
        const dt = ticker.deltaMS / 1e3;
        camera.update(dt);
        decor?.update(dt);
        const zoom = world.scale.x;
        for (const rt of runtimes.values()) {
          rt.character.setBubbleZoom(zoom);
          rt.character.update(dt);
        }
        updateCafeteria(dt);
        updateCoffeeRuns(dt);
        updateErrands(dt);
        updateBossAura(dt);
        updateDeskLife(dt);
        updateBoardMoves(dt);
        resolveBubbleOverlaps();
        for (let i = envelopes.length - 1; i >= 0; i--) {
          if (envelopes[i].update(dt)) {
            envelopes[i].destroy();
            envelopes.splice(i, 1);
          }
        }
      };
      app.ticker.add(onTick);
      app.ticker.maxFPS = FLOOR_MAX_FPS;
      app.__unannounce = announceWhenWhole(
        app.ticker,
        () => pendingAdds.size === 0 && useStore.getState().agents.every((a) => runtimes.has(a.id)),
        () => {
          if (mountIdRef.current === mountId) spotsRef.current?.(readSpot);
        }
      );
      applyFloorPause([app.ticker, Ticker.system], pausedRef.current);
      drawSettledWhilePaused(app.ticker, camera, pausedRef.current);
      const resize = new ResizeObserver((entries) => {
        for (const e of entries) {
          const { width, height } = e.contentRect;
          if (width === 0 || height === 0) continue;
          app.renderer?.resize(width, height);
          camera.setViewSize(width, height);
          drawSettledWhilePaused(app.ticker, camera, pausedRef.current);
        }
      });
      resize.observe(host);
      app.__resize = resize;
      initRetriesRef.current = 0;
    };
    init().catch((err) => {
      if (mountIdRef.current !== mountId) return;
      const plan = planInitFailure(err, initRetriesRef.current);
      if (plan.action === "retry") {
        initRetriesRef.current = plan.attempt;
        console.warn(`[OfficeFloor] could not get a WebGL context (the GPU process may be restarting) — retrying, attempt ${plan.attempt}/${DEFAULT_MAX_INIT_RETRIES}`);
        setTimeout(() => {
          if (mountIdRef.current === mountId) setGlGeneration((n) => n + 1);
        }, plan.delayMs);
        return;
      }
      if (plan.action === "give-up") {
        console.error(`[OfficeFloor] still no WebGL context after ${DEFAULT_MAX_INIT_RETRIES} retries:`, err);
        cannotDrawRef.current?.();
        host.appendChild(floorNote(
          "The office floor could not get a GPU context.\n\nThe GPU may still be restarting, or too many terminals are\nusing it at once. Close a few agent terminals, or restart\nthe app, to bring the floor back."
        ));
        return;
      }
      console.error("[OfficeFloor] init failed:", err);
      cannotDrawRef.current?.();
      host.appendChild(floorNote(
        "OfficeFloor failed to start:\n" + (err?.stack || err?.message || String(err))
      ));
    });
    return () => {
      mountIdRef.current++;
      const a = appRef.current;
      spotsRef.current?.(null);
      if (a) {
        a.__glRecovery?.();
        a.__resize?.disconnect?.();
        try {
          a.__unannounce?.();
        } catch {
        }
        try {
          a.__unsub?.();
        } catch {
        }
        try {
          a.__unsubAsleep?.();
        } catch {
        }
        try {
          a.__offMessage?.();
        } catch {
        }
        try {
          clearInterval(a.__taskBoardPoll);
        } catch {
        }
        try {
          a.__unsubAvatars?.();
        } catch {
        }
        try {
          a.__unsubSeason?.();
        } catch {
        }
        safeDestroy(a);
      }
      appRef.current = null;
      taskBoardPollRef.current = null;
      try {
        Ticker.system.start();
      } catch {
      }
      while (host.firstChild) host.removeChild(host.firstChild);
    };
  }, [officeTheme, glGeneration, i18n.language]);
  return /* @__PURE__ */ jsxRuntimeExports.jsx(FloorEdge, { fill, children: (inner) => {
    const r = fitRect(inner.w, inner.h, shape.w, shape.h);
    offsetRef.current = { x: inner.inset + r.x, y: inner.inset + r.y };
    return /* @__PURE__ */ jsxRuntimeExports.jsx("div", { ref: hostRef, style: { position: "absolute", left: r.x, top: r.y, width: r.w, height: r.h, overflow: "hidden", imageRendering: "pixelated" } });
  } });
}
function floorNote(text) {
  const note = document.createElement("div");
  note.style.cssText = "position:absolute;inset:0;display:flex;align-items:center;justify-content:center;padding:24px;color:#ffd0b5;font-family:monospace;font-size:13px;text-align:center;white-space:pre-wrap;";
  note.textContent = text;
  return note;
}
function hexNum(n) {
  return n;
}
function hex(n) {
  return "#" + n.toString(16).padStart(6, "0");
}
function safeDestroy(app) {
  try {
    app.ticker?.stop();
  } catch {
  }
  try {
    app.destroy(true, { children: true });
  } catch {
  }
}
export {
  OfficeFloor
};
