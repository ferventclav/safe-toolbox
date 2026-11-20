//! Windows notifications for an unpackaged exe: an AppUserModelID registered under HKCU (name and icon in the
//! notification), and a "safe-toolbox:" link so clicking a notification opens the right page.

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

pub const AUMID: &str = "io.github.ferventclav.safetoolbox";
pub const SCHEME: &str = "safe-toolbox";

/// Registers the notification identity and the link scheme for this exe. Cheap; run at every start so a moved
/// exe keeps working.
pub fn register(exe: &str, icon_png: Option<&str>) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (id, _) = hkcu.create_subkey(format!(r"Software\Classes\AppUserModelId\{AUMID}"))?;
    id.set_value("DisplayName", &"Safe Toolbox")?;
    if let Some(icon) = icon_png {
        id.set_value("IconUri", &icon)?;
    }
    let (scheme, _) = hkcu.create_subkey(format!(r"Software\Classes\{SCHEME}"))?;
    scheme.set_value("", &"URL:Safe Toolbox")?;
    scheme.set_value("URL Protocol", &"")?;
    let (cmd, _) = scheme.create_subkey(r"shell\open\command")?;
    cmd.set_value("", &format!("\"{exe}\" --open \"%1\""))?;
    Ok(())
}

/// The page a "safe-toolbox:updates" link asks for.
pub fn page_from_link(link: &str) -> Option<String> {
    let rest = link.strip_prefix(SCHEME)?.strip_prefix(':')?.trim_matches('/');
    let page: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    (!page.is_empty()).then_some(page)
}

/// Shows a notification; clicking it opens `page`.
pub fn show(title: &str, body: &str, page: &str) -> Result<(), String> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    let esc = super::xml_escape;
    let xml = format!(
        "<toast activationType=\"protocol\" launch=\"{SCHEME}:{}\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        esc(page),
        esc(title),
        esc(body)
    );
    let go = || -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
    };
    go().map_err(|e| e.message().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn links_name_a_page() {
        assert_eq!(super::page_from_link("safe-toolbox:updates").as_deref(), Some("updates"));
        assert_eq!(super::page_from_link("safe-toolbox://clean-up/").as_deref(), Some("clean-up"));
        assert_eq!(super::page_from_link("other:updates"), None);
    }
}
