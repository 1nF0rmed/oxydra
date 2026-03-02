(function () {
  const STEP_LABELS = [
    'Welcome',
    'Runner Configuration',
    'Register First User',
    'Provider Setup',
    'Review',
  ];

  function inferDefaultUserConfigPath(userId) {
    const trimmed = (userId || '').trim();
    if (!trimmed) {
      return 'users/default.toml';
    }
    return `users/${trimmed}.toml`;
  }

  function createOnboardingState() {
    return {
      step: 1,
      runnerWorkspaceRoot: '.oxydra/workspaces',
      userId: 'default',
      userConfigPath: 'users/default.toml',
      providerName: 'openai',
      providerType: 'openai',
      providerApiKey: '',
      providerApiKeyEnv: 'OPENAI_API_KEY',
      providerEnvResolved: false,
      busy: false,
      error: '',
      done: false,
    };
  }

  function stepLabel(step) {
    return STEP_LABELS[step - 1] || 'Setup';
  }

  window.OxydraOnboarding = {
    STEP_LABELS,
    inferDefaultUserConfigPath,
    createOnboardingState,
    stepLabel,
  };
})();
