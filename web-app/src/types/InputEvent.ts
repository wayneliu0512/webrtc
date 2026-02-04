export type InputEvent =
  | { pointermotion: { dx: number; dy: number } }
  | { pointerbutton: { button: number; pressed: boolean } }
  | { scroll: { steps_x: number; steps_y: number } }
  | { key: { keycode: number; pressed: boolean } };
