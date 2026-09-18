// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  AttrBinding, CondBinding, RenderBinding, RepeatBinding, TemplateInstance, TextBinding,
} from './types.js';

/** Fragment limits apply to one whole mount/update, not individual structural blocks. */
export const MAX_FRAGMENT_DEPTH = 256;
export const MAX_FRAGMENT_VISITS = 100_000;

export type FragmentTask = (
  | TemplateInstance | TextBinding | AttrBinding | CondBinding | RepeatBinding | RenderBinding
) & { queuedGeneration?: number };

/** Host adapter keeps the reusable work stack independent of DOM operations. */
export interface FragmentWorkHost {
  $processFragmentTask(task: FragmentTask, requireKnownState?: boolean): void;
}

/** Reusable operation-owned stack; nested blocks never reset counters or recurse. */
export interface FragmentWork {
  readonly stack: FragmentTask[];
  active: boolean;
  generation: number;
  visits: number;
  /** Keep one mount budget across hydration and deferred state replay passes. */
  holdBudget(): void;
  releaseBudget(): void;
  begin(): boolean;
  nextPass(): void;
  enqueue(task: FragmentTask): void;
  sort(): void;
  visit(depth: number): void;
  drain(host: FragmentWorkHost, requireKnownState?: boolean): void;
  end(): void;
}

let fragmentWorkConstructor: (new () => FragmentWork) | undefined;

/** Create isolated work state, defining the shared implementation only on first use. */
export function createFragmentWork(): FragmentWork {
  if (!fragmentWorkConstructor) {
    function ownerOrder(task: FragmentTask): number {
      return ('texts' in task ? task : task.owner)?.order ?? 0;
    }

    function compareTasks(left: FragmentTask, right: FragmentTask): number {
      return ownerOrder(right) - ownerOrder(left);
    }

    fragmentWorkConstructor = class FragmentWorkImplementation implements FragmentWork {
      readonly stack: FragmentTask[] = [];
      active = false;
      generation = 0;
      visits = 0;
      private budgetHolds = 0;

      holdBudget(): void {
        if (this.budgetHolds++ === 0 && !this.active) this.visits = 0;
      }

      releaseBudget(): void {
        this.budgetHolds--;
      }

      begin(): boolean {
        if (this.active) return false;
        this.active = true;
        if (this.budgetHolds === 0) this.visits = 0;
        this.generation++;
        return true;
      }

      nextPass(): void {
        this.generation++;
      }

      enqueue(task: FragmentTask): void {
        if (task.generation === this.generation || task.queuedGeneration === this.generation) return;
        task.queuedGeneration = this.generation;
        this.stack.push(task);
      }

      sort(): void {
        this.stack.sort(compareTasks);
      }

      visit(depth: number): void {
        if (depth > MAX_FRAGMENT_DEPTH) {
          throw new Error(`[WebUI] Fragment call depth exceeds ${MAX_FRAGMENT_DEPTH}; terminate recursive renders.`);
        }
        if (++this.visits > MAX_FRAGMENT_VISITS) {
          throw new Error(`[WebUI] Fragment invocation visits exceed ${MAX_FRAGMENT_VISITS} in one update; reduce rendered work.`);
        }
      }

      drain(host: FragmentWorkHost, requireKnownState?: boolean): void {
        while (this.stack.length) {
          const task = this.stack.pop()!;
          const owner = 'texts' in task ? task : task.owner;
          if (owner?.alive === false || task.generation === this.generation) continue;
          task.generation = this.generation;
          host.$processFragmentTask(task, requireKnownState);
        }
      }

      end(): void {
        this.stack.length = 0;
        this.active = false;
      }
    };
  }
  return new fragmentWorkConstructor();
}
