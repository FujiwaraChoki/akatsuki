// RN's Modal with the motion rule of the spec (§2.9): sheets slide (menus fade) unless the system asks for reduced
// motion, then they just appear.
import type { ComponentProps } from "react";
import { Modal } from "react-native";
import { useReducedMotion } from "react-native-reanimated";

export function SheetModal({
  fade,
  ...props
}: Omit<ComponentProps<typeof Modal>, "animationType"> & { fade?: boolean }) {
  const reduced = useReducedMotion();
  return <Modal {...props} animationType={reduced ? "none" : fade ? "fade" : "slide"} />;
}
