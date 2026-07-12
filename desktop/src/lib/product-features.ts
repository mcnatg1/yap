export function isDevelopmentPolishAvailable({
  explicitlyEnabled,
  isDevelopment,
}: {
  explicitlyEnabled: boolean;
  isDevelopment: boolean;
}) {
  return isDevelopment && explicitlyEnabled;
}

export const developmentPolishAvailable = isDevelopmentPolishAvailable({
  explicitlyEnabled: import.meta.env.VITE_ENABLE_DEVELOPMENT_POLISH === "true",
  isDevelopment: import.meta.env.DEV,
});
