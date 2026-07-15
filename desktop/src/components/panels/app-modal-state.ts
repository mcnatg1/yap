export function projectAppModalState(modal: "settings" | "help" | null) {
  return {
    detailsOpen: modal === "settings",
    helpOpen: modal === "help",
  };
}
