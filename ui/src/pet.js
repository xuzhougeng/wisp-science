const ROWS = {
  idle: 0,
  "running-right": 1,
  "running-left": 2,
  waving: 3,
  jumping: 4,
  failed: 5,
  waiting: 6,
  running: 7,
  review: 8,
};

const DEFAULT_COUNTS = {
  idle: 7,
  "running-right": 8,
  "running-left": 8,
  waving: 4,
  jumping: 5,
  failed: 8,
  waiting: 6,
  running: 6,
  review: 6,
};

class PetRuntime {
  constructor(element) {
    this.element = element;
    this.sprite = element.querySelector(".wisp-pet-sprite");
    this.frame = 0;
    this.state = "idle";
    this.sequence = -1;
    this.x = 0;
    this.timer = 0;
    this.roamTimer = 0;
    this.pointerTimer = 0;
    this.oneShot = false;
    this.reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
    this.onPointerMove = this.onPointerMove.bind(this);
    this.onClick = this.onClick.bind(this);
    document.addEventListener("pointermove", this.onPointerMove, { passive: true });
    element.addEventListener("click", this.onClick);
  }

  configure(config) {
    this.config = config;
    this.element.classList.toggle("is-visible", !!config.visible && !!config.src);
    this.element.title = config.name || "Pet";
    if (this.src !== config.src) {
      this.src = config.src;
      this.sprite.style.backgroundImage = config.src ? `url("${config.src}")` : "none";
    }
    if (this.configuredState !== config.state || this.configuredSequence !== config.sequence) {
      this.configuredState = config.state;
      this.configuredSequence = config.sequence;
      this.externalState = config.state;
      this.sequence = config.sequence;
      this.play(config.state || "idle", ["waving", "jumping", "failed"].includes(config.state));
    }
  }

  frameCount(state) {
    const count = Number(this.config?.frameCounts?.[state] ?? DEFAULT_COUNTS[state] ?? 1);
    return Math.max(1, Math.min(8, count));
  }

  paint(row, column) {
    const x = (Math.max(0, Math.min(7, column)) * 100) / 7;
    const y = (Math.max(0, Math.min(10, row)) * 100) / 10;
    this.sprite.style.backgroundPosition = `${x}% ${y}%`;
  }

  play(state, oneShot = false) {
    clearInterval(this.timer);
    clearTimeout(this.roamTimer);
    this.state = state in ROWS ? state : "idle";
    this.oneShot = oneShot;
    this.looking = false;
    this.frame = 0;
    this.element.dataset.state = this.state;
    this.paint(ROWS[this.state], 0);
    if (this.reduceMotion) {
      if (oneShot) {
        setTimeout(() => {
          if (this.oneShot) {
            this.externalState = "idle";
            this.play("idle");
          }
        }, 500);
      }
      if (this.state === "idle") this.scheduleRoam();
      return;
    }
    const count = this.frameCount(this.state);
    const interval = this.state === "idle" ? 170 : 125;
    this.timer = setInterval(() => {
      if (!this.element.isConnected) return this.destroy();
      if (this.looking) return;
      this.frame += 1;
      if (this.oneShot && this.frame >= count) {
        clearInterval(this.timer);
        const linger = this.state === "failed" ? 700 : 80;
        setTimeout(() => {
          if (this.oneShot) {
            this.externalState = "idle";
            this.play("idle");
          }
        }, linger);
        return;
      }
      this.paint(ROWS[this.state], this.frame % count);
    }, interval);
    if (this.state === "idle") this.scheduleRoam();
  }

  scheduleRoam() {
    clearTimeout(this.roamTimer);
    if (this.reduceMotion || this.externalState !== "idle") return;
    this.roamTimer = setTimeout(() => this.roam(), 5000 + Math.random() * 6000);
  }

  roam() {
    if (this.externalState !== "idle" || this.oneShot || !this.element.classList.contains("is-visible")) {
      return this.scheduleRoam();
    }
    const limit = Math.min(360, Math.max(80, innerWidth * 0.32));
    const target = -Math.round(Math.random() * limit);
    const distance = Math.abs(target - this.x);
    if (distance < 45) return this.scheduleRoam();
    const duration = Math.max(650, Math.min(2100, distance * 8));
    this.element.style.setProperty("--pet-walk-ms", `${duration}ms`);
    this.element.style.setProperty("--pet-x", `${target}px`);
    this.play(target > this.x ? "running-right" : "running-left");
    this.x = target;
    clearTimeout(this.roamTimer);
    this.roamTimer = setTimeout(() => this.play("idle"), duration);
  }

  onPointerMove(event) {
    if (this.reduceMotion || this.externalState !== "idle" || this.oneShot || this.state !== "idle") return;
    const rect = this.element.getBoundingClientRect();
    const dx = event.clientX - (rect.left + rect.width / 2);
    const dy = event.clientY - (rect.top + rect.height / 2);
    const distance = Math.hypot(dx, dy);
    clearTimeout(this.pointerTimer);
    if (distance < 42 || distance > 380) return;
    const degrees = (Math.atan2(dx, -dy) * 180 / Math.PI + 360) % 360;
    const direction = Math.round(degrees / 22.5) % 16;
    this.paint(direction < 8 ? 9 : 10, direction % 8);
    this.looking = true;
    this.element.dataset.state = "looking";
    this.pointerTimer = setTimeout(() => {
      if (this.state === "idle") {
        this.looking = false;
        this.element.dataset.state = "idle";
        this.paint(ROWS.idle, this.frame % this.frameCount("idle"));
      }
    }, 900);
  }

  onClick(event) {
    event.stopPropagation();
    if (this.externalState === "idle" && !this.oneShot) {
      this.play("waving", true);
    }
  }

  destroy() {
    clearInterval(this.timer);
    clearTimeout(this.roamTimer);
    clearTimeout(this.pointerTimer);
    document.removeEventListener("pointermove", this.onPointerMove);
    this.element.removeEventListener("click", this.onClick);
  }
}

export function sync_pet(elementId, configJson) {
  const config = JSON.parse(configJson);
  const sync = () => {
    const element = document.getElementById(elementId);
    if (!element) return;
    if (!element.__wispPetRuntime) element.__wispPetRuntime = new PetRuntime(element);
    element.__wispPetRuntime.configure(config);
  };
  requestAnimationFrame(sync);
}
