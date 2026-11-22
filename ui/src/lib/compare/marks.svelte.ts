import type { Mark } from './quick';

/**
 * What a comparison marked in one pane, by entry name. A plain map replaced as a whole (a folder can hold 100 000
 * entries); readers track `#revision`. A pane clears it when it lists anything.
 */
export class CompareMarks {
  #marks = new Map<string, Mark>();
  #revision = $state(0);

  get(name: string): Mark | undefined {
    void this.#revision;
    return this.#marks.get(name);
  }

  set(marks: Map<string, Mark>) {
    this.#marks = marks;
    this.#revision += 1;
  }

  clear() {
    if (this.#marks.size > 0) {
      this.set(new Map());
    }
  }
}
