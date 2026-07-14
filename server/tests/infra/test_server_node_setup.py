import os
import shutil
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SETUP_SCRIPT = REPO_ROOT / "infra" / "yap-server-node" / "setup-server.sh"
ENV_EXAMPLE = REPO_ROOT / "infra" / "yap-server-node" / "server.env.example"
SCRIPT_ARGUMENT = SETUP_SCRIPT.relative_to(REPO_ROOT).as_posix()


def _find_bash() -> str | None:
    if os.name == "nt":
        candidate = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git" / "bin" / "bash.exe"
        if candidate.is_file():
            return str(candidate)
    return shutil.which("bash")


def _run_bash(
    *arguments: str,
    env: dict[str, str] | None = None,
    input_text: str | None = None,
) -> subprocess.CompletedProcess[str]:
    bash = _find_bash()
    if bash is None:
        raise unittest.SkipTest("bash is unavailable")
    process_env = os.environ.copy()
    if env:
        process_env.update(env)
    return subprocess.run(
        [bash, *arguments],
        check=False,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        env=process_env,
        input=input_text,
    )


class ServerNodeSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.script = SETUP_SCRIPT.read_text(encoding="utf-8")
        cls.env_example = ENV_EXAMPLE.read_text(encoding="utf-8")

    def test_setup_script_is_bash_syntax_valid(self) -> None:
        completed = _run_bash("-n", SCRIPT_ARGUMENT)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_destructive_setup_behaviors_are_opt_in(self) -> None:
        self.assertIn(': "${YAP_FIREWALL_RESET:=0}"', self.script)
        self.assertIn(': "${YAP_DISABLE_NOISE_SERVICES:=0}"', self.script)
        self.assertNotIn(': "${YAP_FIREWALL_RESET:=1}"', self.script)
        self.assertNotIn(': "${YAP_DISABLE_NOISE_SERVICES:=1}"', self.script)
        self.assertIn("YAP_FIREWALL_RESET=0", self.env_example)
        self.assertIn("YAP_DISABLE_NOISE_SERVICES=0", self.env_example)

    def test_network_exposure_defaults_are_closed(self) -> None:
        self.assertIn(': "${YAP_LAN_SSH_CIDR=}"', self.script)
        self.assertIn(': "${YAP_APP_PORT=}"', self.script)
        self.assertIn(': "${YAP_APP_CIDR=}"', self.script)
        self.assertIn("YAP_LAN_SSH_CIDR=\n", self.env_example)
        self.assertIn("YAP_APP_PORT=\n", self.env_example)
        self.assertIn("YAP_APP_CIDR=\n", self.env_example)

    def test_phase_3_port_is_tunnel_only_by_default(self) -> None:
        expected = "3389 5909 11000 11434 18765"
        self.assertIn(f': "${{YAP_TUNNEL_ONLY_PORTS:={expected}}}"', self.script)
        self.assertIn(f'YAP_TUNNEL_ONLY_PORTS="{expected}"', self.env_example)

    def test_validate_only_accepts_closed_defaults_without_root(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={"YAP_VALIDATE_ONLY": "1"},
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("configuration is valid", completed.stdout)

    def test_validate_only_runs_when_script_arrives_on_stdin(self) -> None:
        completed = _run_bash(
            "-s",
            env={"YAP_VALIDATE_ONLY": "1"},
            input_text=self.script,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("configuration is valid", completed.stdout)

    def test_invalid_app_config_fails_during_non_mutating_validation(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_APP_PORT": "443",
                "YAP_APP_CIDR": "",
                "YAP_ZSCALER_APP_CIDR": "",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("no YAP_APP_CIDR or YAP_ZSCALER_APP_CIDR", completed.stderr)

    def test_direct_app_port_cannot_overlap_tunnel_only_port(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_APP_PORT": "18765",
                "YAP_APP_CIDR": "192.168.50.63/32",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("also listed in YAP_TUNNEL_ONLY_PORTS", completed.stderr)

    def test_invalid_firewall_source_fails_during_non_mutating_validation(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_LAN_SSH_CIDR": "not-a-network",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("YAP_LAN_SSH_CIDR must be a valid", completed.stderr)

    def test_effective_policy_address_must_be_a_representative_management_ip(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_PRIVATE_SSH_FROM": "",
                "YAP_LAN_SSH_CIDR": "10.20.0.0/16",
                "YAP_SSH_POLICY_TEST_ADDR": "192.168.50.63",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("must fall within a configured SSH management source", completed.stderr)

    def test_effective_policy_requires_at_least_one_management_source(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_PRIVATE_SSH_FROM": "",
                "YAP_LAN_SSH_CIDR": "",
                "YAP_OVERLAY_SSH_CIDR": "",
                "YAP_ZSCALER_SSH_CIDR": "",
                "YAP_SSH_POLICY_TEST_ADDR": "192.168.50.63",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("must fall within a configured SSH management source", completed.stderr)

    def test_effective_ssh_policy_rejects_earlier_weak_values(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
select_validation_python
YAP_OWNER=admin
YAP_SSH_POLICY_TEST_ADDR=192.168.50.63
sshd() {{
  cat <<'POLICY'
authenticationmethods publickey
allowusers admin
allowagentforwarding no
permittunnel no
permituserenvironment no
allowtcpforwarding local
gatewayports no
permitrootlogin no
passwordauthentication yes
pubkeyauthentication yes
kbdinteractiveauthentication no
x11forwarding no
POLICY
}}
verify_effective_ssh_policy
"""
        completed = _run_bash("-c", harness)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("passwordauthentication", completed.stderr)

    def test_effective_ssh_policy_rejects_additive_allowusers(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
select_validation_python
YAP_OWNER=admin
YAP_SSH_POLICY_TEST_ADDR=192.168.50.63
sshd() {{
  cat <<'POLICY'
authenticationmethods publickey
allowusers admin unexpected
allowagentforwarding no
permittunnel no
permituserenvironment no
allowtcpforwarding local
gatewayports no
permitrootlogin no
passwordauthentication no
pubkeyauthentication yes
kbdinteractiveauthentication no
x11forwarding no
POLICY
}}
verify_effective_ssh_policy
"""
        completed = _run_bash("-c", harness)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("allowusers", completed.stderr)

    def test_effective_policy_derives_a_context_for_every_management_source(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
select_validation_python
YAP_SSH_POLICY_TEST_ADDR=192.168.50.63
YAP_PRIVATE_SSH_FROM=192.168.50.63
YAP_LAN_SSH_CIDR=10.20.0.0/16
YAP_OVERLAY_SSH_CIDR=100.64.12.0/24
YAP_ZSCALER_SSH_CIDR=
management_policy_addresses
"""
        completed = _run_bash("-c", harness)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            completed.stdout.splitlines(),
            ["192.168.50.63", "10.20.0.1", "100.64.12.1"],
        )

    def test_failed_private_profile_activation_is_fatal(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
YAP_CONFIGURE_PRIVATE_ETHERNET=1
nmcli() {{
  if [ "$1" = "-t" ]; then
    return 0
  fi
  if [ "$1 $2" = "con up" ]; then
    return 37
  fi
  return 0
}}
setup_private_ethernet
"""
        completed = _run_bash("-c", harness)
        self.assertEqual(completed.returncode, 37)

    def test_firewall_reset_requires_local_console_acknowledgement(self) -> None:
        completed = _run_bash(
            SCRIPT_ARGUMENT,
            env={
                "YAP_VALIDATE_ONLY": "1",
                "YAP_FIREWALL_RESET": "1",
                "YAP_FIREWALL_RESET_CONFIRM": "",
            },
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("YAP_FIREWALL_RESET_CONFIRM=local-console", completed.stderr)

    def test_failed_recovery_rule_cannot_report_ufw_reenabled(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
YAP_PRIVATE_SSH_FROM=192.168.50.63
YAP_LAN_SSH_CIDR=
YAP_OVERLAY_SSH_CIDR=
YAP_ZSCALER_SSH_CIDR=
FIREWALL_RESET_IN_PROGRESS=1
ufw() {{
  if [ "$1" = "allow" ]; then
    return 37
  fi
  return 0
}}
false
"""
        completed = _run_bash("-c", harness)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("automatic UFW recovery failed", completed.stderr)
        self.assertNotIn("UFW was re-enabled", completed.stderr)

    def test_missing_private_management_address_is_fatal(self) -> None:
        harness = f"""
source {SCRIPT_ARGUMENT!s}
YAP_PRIVATE_IFACE=enP7s7
YAP_PRIVATE_ADDR=192.168.50.1/24
YAP_PRIVATE_SSH_FROM=192.168.50.63
ip() {{ return 1; }}
verify_private_management_address
"""
        completed = _run_bash("-c", harness)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("Private management address", completed.stderr)

    def test_validation_precedes_every_host_mutation(self) -> None:
        main = self.script[self.script.index("main() {") :]
        self.assertLess(main.index("validate_config"), main.index("need_root"))
        self.assertLess(main.index("validate_config"), main.index("install_basics"))
        self.assertLess(
            main.index("verify_private_management_address"),
            main.index("setup_firewall"),
        )
        firewall = self.script[self.script.index("setup_firewall() {") :]
        self.assertLess(
            firewall.index("apply_management_ssh_rules"),
            firewall.index("ufw --force enable"),
        )
        ssh = self.script[self.script.index("setup_ssh() {") :]
        self.assertLess(
            ssh.index("verify_effective_ssh_policy"),
            ssh.index("systemctl reload ssh"),
        )


if __name__ == "__main__":
    unittest.main()
