//! Serde shapes of the Microsoft Graph v1.0 resources the adapter reads. Only the fields the
//! adapter selects are modelled; everything is optional because Graph omits unselected or empty
//! properties and sometimes sends `null` for collections.

use serde::Deserialize;
use serde_json::Value;

/// A collection page (`value` plus paging links).
#[derive(Debug, Deserialize)]
pub(crate) struct Page<T> {
    #[serde(default = "Vec::new")]
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

/// `emailAddress`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// `recipient`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Recipient {
    pub email_address: Option<EmailAddress>,
}

/// `followupFlag`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Flag {
    pub flag_status: Option<String>,
}

/// `internetMessageHeader`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Header {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

/// `message` (the selected subset), also used for delta entries.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Message {
    pub id: String,
    pub conversation_id: Option<String>,
    pub internet_message_id: Option<String>,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub from: Option<Recipient>,
    pub sender: Option<Recipient>,
    pub to_recipients: Option<Vec<Recipient>>,
    pub cc_recipients: Option<Vec<Recipient>>,
    pub bcc_recipients: Option<Vec<Recipient>>,
    pub reply_to: Option<Vec<Recipient>>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub flag: Option<Flag>,
    pub categories: Option<Vec<String>>,
    pub has_attachments: Option<bool>,
    pub parent_folder_id: Option<String>,
    pub internet_message_headers: Option<Vec<Header>>,
    /// Present on delta entries for messages that left the folder (`{"reason": "deleted"}`).
    #[serde(rename = "@removed")]
    pub removed: Option<Value>,
}

impl Message {
    /// Unread at the provider.
    pub fn unread(&self) -> bool {
        self.is_read == Some(false)
    }

    /// Flagged for follow-up (`sys:starred`).
    pub fn flagged(&self) -> bool {
        self.flag.as_ref().and_then(|f| f.flag_status.as_deref()).is_some_and(|s| s.eq_ignore_ascii_case("flagged"))
    }

    /// A draft.
    pub fn draft(&self) -> bool {
        self.is_draft == Some(true)
    }

    /// The categories (display names).
    pub fn category_names(&self) -> &[String] {
        self.categories.as_deref().unwrap_or(&[])
    }
}

/// `mailFolder` (selected subset).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Folder {
    pub id: String,
    pub display_name: Option<String>,
    pub parent_folder_id: Option<String>,
    pub child_folder_count: Option<u32>,
    pub is_hidden: Option<bool>,
}

/// `outlookCategory` (a master category).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Category {
    pub id: String,
    pub display_name: String,
    pub color: Option<String>,
}

/// `subscription` (selected subset).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Subscription {
    pub id: String,
    pub expiration_date_time: Option<String>,
}

/// `uploadSession`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadSession {
    pub upload_url: String,
}

/// `user` (the `/me` subset used at connect time).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct User {
    pub id: String,
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}
