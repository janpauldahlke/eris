/// Describes a single API endpoint for agent consumption.
#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    pub id: &'static str,
    pub service: &'static str,
    pub resource_path: &'static str,
    pub method_name: &'static str,
    pub http_method: &'static str,
    pub description: &'static str,
    pub path_template: &'static str,
    pub base_url: &'static str,
    pub scopes: &'static [&'static str],
    pub parameters: &'static [ParamDescriptor],
    pub request_body_schema: Option<&'static str>,
    pub response_body_schema: Option<&'static str>,
    pub supports_pagination: bool,
    pub supports_media_upload: bool,
    pub supports_media_download: bool,
    pub deprecated: bool,
}
/// Describes a single parameter on an action.
#[derive(Debug, Clone)]
pub struct ParamDescriptor {
    pub name: &'static str,
    pub param_type: &'static str,
    pub location: &'static str,
    pub required: bool,
    pub description: &'static str,
    pub default_value: Option<&'static str>,
    pub enum_values: Option<&'static [&'static str]>,
    pub deprecated: bool,
}
///A single MIME message part.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    ///The MIME type of the message part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    ///The immutable ID of the message part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<String>,
    ///The filename of the attachment. Only present if this message part represents an attachment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    ///List of headers on this message part. For the top-level message part, representing the entire message payload, it will contain the standard RFC 2822 email headers such as `To`, `From`, and `Subject`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<MessagePartHeader>>,
    ///The message part body for this part, which may be empty for container MIME message parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<MessagePartBody>,
    ///The child MIME message parts of this part. This only applies to container MIME message parts, for example `multipart/*`. For non- container MIME message part types, such as `text/plain`, this field is empty. For more information, see RFC 1521.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Box<Vec<MessagePart>>>,
}
///The body of a single MIME message part.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePartBody {
    ///The body data of a MIME message part as a base64url encoded string. May be empty for MIME container types that have no message body or when the body data is sent as a separate attachment. An attachment ID is present if the body data is contained in a separate attachment.
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::deserialize_bytes_base64"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    ///When present, contains the ID of an external attachment that can be retrieved in a separate `messages.attachments.get` request. When not present, the entire content of the message part body is contained in the data field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    ///Number of bytes for the message part data (encoding notwithstanding).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i32>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePartHeader {
    ///The value of the header after the `:` separator. For example, `someuser@example.com`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    ///The name of the header before the `:` separator. For example, `To`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
///An email message.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    ///Classification Label values on the message. Available Classification Label schemas can be queried using the Google Drive Labels API. Each classification label ID must be unique. If duplicate IDs are provided, only one will be retained, and the selection is arbitrary. Only used for Google Workspace accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_label_values: Option<Vec<ClassificationLabelValue>>,
    ///The ID of the thread the message belongs to. To add a message or draft to a thread, the following criteria must be met: 1. The requested `threadId` must be specified on the `Message` or `Draft.Message` you supply with your request. 2. The `References` and `In-Reply-To` headers must be set in compliance with the [RFC 2822](https://tools.ietf.org/html/rfc2822) standard. 3. The `Subject` headers must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    ///Estimated size in bytes of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_estimate: Option<i32>,
    ///The entire email message in an RFC 2822 formatted and base64url encoded string. Returned in `messages.get` and `drafts.get` responses when the `format=RAW` parameter is supplied.
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::deserialize_bytes_base64"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Vec<u8>>,
    ///The ID of the last history record that modified this message.
    #[serde(default, deserialize_with = "super::serde_helpers::string_to_u64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<u64>,
    ///The immutable ID of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    ///A short part of the message text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    ///The parsed email structure in the message parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<MessagePart>,
    ///The internal message creation timestamp (epoch ms), which determines ordering in the inbox. For normal SMTP-received email, this represents the time the message was originally accepted by Google, which is more reliable than the `Date` header. However, for API-migrated mail, it can be configured by client to be based on the `Date` header.
    #[serde(default, deserialize_with = "super::serde_helpers::string_to_i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_date: Option<i64>,
    ///List of IDs of labels applied to this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
}
///Classification Labels applied to the email message. Classification Labels are different from Gmail inbox labels. Only used for Google Workspace accounts. [Learn more about classification labels](https://support.google.com/a/answer/9292382).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationLabelValue {
    ///Field values for the given classification label ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<ClassificationLabelFieldValue>>,
    ///Required. The canonical or raw alphanumeric classification label ID. Maps to the ID field of the Google Drive Label resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_id: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesResponse {
    ///List of messages. Note that each message resource contains only an `id` and a `threadId`. Additional message details can be fetched using the messages.get method.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Message>>,
    ///Token to retrieve the next page of results in the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    ///Estimated total number of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_size_estimate: Option<u32>,
}
///Field values for a classification label.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationLabelFieldValue {
    ///Required. The field ID for the Classification Label Value. Maps to the ID field of the Google Drive `Label.Field` object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    ///Selection choice ID for the selection option. Should only be set if the field type is `SELECTION` in the Google Drive `Label.Field` object. Maps to the id field of the Google Drive `Label.Field.SelectionOptions` resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}
///Query/path parameters for `gmail.users.messages.get`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_headers: Option<String>,
}
///Query/path parameters for `gmail.users.messages.list`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_spam_trash: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
}
///Query/path parameters for `gmail.users.messages.send`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}
pub static GET_ACTION: ActionDescriptor = ActionDescriptor {
    id: "gmail.users.messages.get",
    service: "gmail",
    resource_path: "users.messages",
    method_name: "get",
    http_method: "GET",
    description: "Gets the specified message.",
    path_template: "gmail/v1/users/{userId}/messages/{id}",
    base_url: "https://gmail.googleapis.com/",
    scopes: &[
        "https://mail.google.com/",
        "https://www.googleapis.com/auth/gmail.addons.current.message.action",
        "https://www.googleapis.com/auth/gmail.addons.current.message.metadata",
        "https://www.googleapis.com/auth/gmail.addons.current.message.readonly",
        "https://www.googleapis.com/auth/gmail.metadata",
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.readonly",
    ],
    parameters: &[
        ParamDescriptor {
            name: "userId",
            param_type: "string",
            location: "path",
            required: true,
            description: "The user's email address. The special value `me` can be used to indicate the authenticated user.",
            default_value: Some("\"me\""),
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "id",
            param_type: "string",
            location: "path",
            required: true,
            description: "The ID of the message to retrieve. This ID is usually retrieved using `messages.list`. The ID is also contained in the result when a message is inserted (`messages.insert`) or imported (`messages.import`).",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "format",
            param_type: "string",
            location: "query",
            required: false,
            description: "The format to return the message in.",
            default_value: Some("\"full\""),
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "metadataHeaders",
            param_type: "string",
            location: "query",
            required: false,
            description: "When given and format is `METADATA`, only include headers specified.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: None,
    response_body_schema: Some("Message"),
    supports_pagination: false,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static LIST_ACTION: ActionDescriptor = ActionDescriptor {
    id: "gmail.users.messages.list",
    service: "gmail",
    resource_path: "users.messages",
    method_name: "list",
    http_method: "GET",
    description: "Lists the messages in the user's mailbox. For example usage, see [List Gmail messages](https://developers.google.com/workspace/gmail/api/guides/list-messages).",
    path_template: "gmail/v1/users/{userId}/messages",
    base_url: "https://gmail.googleapis.com/",
    scopes: &[
        "https://mail.google.com/",
        "https://www.googleapis.com/auth/gmail.metadata",
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.readonly",
    ],
    parameters: &[
        ParamDescriptor {
            name: "userId",
            param_type: "string",
            location: "path",
            required: true,
            description: "The user's email address. The special value `me` can be used to indicate the authenticated user.",
            default_value: Some("\"me\""),
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxResults",
            param_type: "integer",
            location: "query",
            required: false,
            description: "Maximum number of messages to return. This field defaults to 100. The maximum allowed value for this field is 500.",
            default_value: Some("\"100\""),
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "pageToken",
            param_type: "string",
            location: "query",
            required: false,
            description: "Page token to retrieve a specific page of results in the list.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "labelIds",
            param_type: "string",
            location: "query",
            required: false,
            description: "Only return messages with labels that match all of the specified label IDs. Messages in a thread might have labels that other messages in the same thread don't have. To learn more, see [Manage labels on messages and threads](https://developers.google.com/workspace/gmail/api/guides/labels#manage_labels_on_messages_threads).",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "includeSpamTrash",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Include messages from `SPAM` and `TRASH` in the results.",
            default_value: Some("\"false\""),
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "q",
            param_type: "string",
            location: "query",
            required: false,
            description: "Only return messages matching the specified query. Supports the same query format as the Gmail search box. For example, `\"from:someuser@example.com rfc822msgid: is:unread\"`. Parameter cannot be used when accessing the api using the gmail.metadata scope.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: None,
    response_body_schema: Some("ListMessagesResponse"),
    supports_pagination: true,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static SEND_ACTION: ActionDescriptor = ActionDescriptor {
    id: "gmail.users.messages.send",
    service: "gmail",
    resource_path: "users.messages",
    method_name: "send",
    http_method: "POST",
    description: "Sends the specified message to the recipients in the `To`, `Cc`, and `Bcc` headers. For example usage, see [Sending email](https://developers.google.com/workspace/gmail/api/guides/sending).",
    path_template: "gmail/v1/users/{userId}/messages/send",
    base_url: "https://gmail.googleapis.com/",
    scopes: &[
        "https://mail.google.com/",
        "https://www.googleapis.com/auth/gmail.addons.current.action.compose",
        "https://www.googleapis.com/auth/gmail.compose",
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.send",
    ],
    parameters: &[
        ParamDescriptor {
            name: "userId",
            param_type: "string",
            location: "path",
            required: true,
            description: "The user's email address. The special value `me` can be used to indicate the authenticated user.",
            default_value: Some("\"me\""),
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: Some("Message"),
    response_body_schema: Some("Message"),
    supports_pagination: false,
    supports_media_upload: true,
    supports_media_download: false,
    deprecated: false,
};
pub static ALL_ACTIONS: &[&ActionDescriptor] = &[
    &GET_ACTION,
    &LIST_ACTION,
    &SEND_ACTION,
];
