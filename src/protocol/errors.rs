//! Srun portal error codes mapped to short ASCII messages. Table adapted
//! from the MIT-licensed vouv/srun Go client and translated to English.

pub fn describe(code: &str) -> Option<&'static str> {
    Some(match code {
        "E3001" => "traffic or time quota exhausted",
        "E3002" => "billing policy condition not matched",
        "E3003" => "control policy condition not matched",
        "E3004" => "insufficient balance",
        "E3005" => "billing policy changed while online",
        "E3006" => "control policy changed while online",
        "E3007" => "timeout",
        "E3008" => "too many connections, kicked from online table",
        "E3009" => "proxy behaviour detected",
        "E3010" => "idle timeout (no traffic)",
        "E3101" => "keepalive timeout",
        "E2531" => "user does not exist",
        "E2532" => "interval between two authentications too short",
        "E2533" => "too many attempts, try again later",
        "E2534" => "temporarily disabled for proxy behaviour",
        "E2535" => "authentication system is closed",
        "E2536" => "system licence expired",
        "E2553" => "wrong password",
        "E2601" => "not a dedicated client",
        "E2606" => "user is disabled",
        "E2611" => "mac binding error",
        "E2612" => "mac address is blacklisted",
        "E2613" => "nas port binding error",
        "E2614" => "vlan id binding error",
        "E2615" => "ip binding error",
        "E2616" => "account in arrears",
        "E2620" => "already online",
        "E2806" => "no matching product",
        "E2807" => "no matching billing policy",
        "E2808" => "no matching control policy",
        "E2833" => "abnormal ip address, please renew the lease",
        "E5990" => "incomplete data",
        "E5991" => "invalid parameter",
        "E5992" => "user not found",
        "E5993" => "user already exists",
        "E4001" => "radius dm offline",
        "E4002" => "dhcp dm offline",
        "E4007" => "local offline",
        "E4008" => "virtual offline",
        "E4101" => "kicked offline by radius module",
        "E4102" => "kicked offline by system settings",
        "E4103" => "kicked offline by admin console",
        "E4104" => "kicked offline by self-service",
        "vcode_error" => "captcha error",
        "not_online_error" => "not online",
        "ip_already_online_error" => "already online",
        "logout_ok" => "logged out",
        _ => return None,
    })
}

/// Rejections the portal lifts by itself after a while: E2532 (two
/// authentications too close together) and E2533 (too many attempts).
pub fn is_transient(code: &str) -> bool {
    matches!(code, "E2532" | "E2533")
}

/// Human readable text for a portal reply: `wrong password (E2553)`,
/// `already online (ip_already_online_error)`, or the raw server strings
/// when the code is unknown: `login_error (E9999): server message`.
pub fn explain(ecode: &str, error: &str, error_msg: &str) -> String {
    let has_ecode = !ecode.is_empty() && ecode != "0";
    let code = if has_ecode { ecode } else { error };
    if let Some(d) = describe(code) {
        return format!("{d} ({code})");
    }
    let mut s = String::new();
    if !error.is_empty() {
        s.push_str(error);
    }
    if has_ecode {
        if s.is_empty() {
            s.push_str(ecode);
        } else {
            s.push_str(&format!(" ({ecode})"));
        }
    }
    if !error_msg.is_empty() {
        if s.is_empty() {
            s.push_str(error_msg);
        } else {
            s.push_str(": ");
            s.push_str(error_msg);
        }
    }
    if s.is_empty() {
        s.push_str("unknown error");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown() {
        assert_eq!(
            explain("E2553", "login_error", "..."),
            "wrong password (E2553)"
        );
        assert_eq!(
            explain("0", "not_online_error", ""),
            "not online (not_online_error)"
        );
        assert_eq!(
            explain("", "ip_already_online_error", ""),
            "already online (ip_already_online_error)"
        );
        assert_eq!(
            explain("E9999", "login_error", "msg"),
            "login_error (E9999): msg"
        );
        assert_eq!(explain("0", "", ""), "unknown error");
    }
}
