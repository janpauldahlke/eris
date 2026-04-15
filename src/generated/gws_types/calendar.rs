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
///Information about the event's reminders for the authenticated user. Note that changing reminders does not also change the updated property of the enclosing event.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReminders {
    ///If the event doesn't use the default reminders, this lists the reminders specific to the event, or, if not set, indicates that no reminders are set for this event. The maximum number of override reminders is 5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<EventReminder>>,
    ///Whether the default reminders of the calendar apply to the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_default: Option<bool>,
}
///Extended properties of the event.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventExtendedProperties {
    ///Properties that are private to the copy of the event that appears on this calendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<std::collections::HashMap<String, String>>,
    ///Properties that are shared between copies of the event on other attendees' calendars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<std::collections::HashMap<String, String>>,
}
///The organizer of the event. If the organizer is also an attendee, this is indicated with a separate entry in attendees with the organizer field set to True. To change the organizer, use the move operation. Read-only, except when importing an event.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventOrganizer {
    ///The organizer's email address, if available. It must be a valid email address as per RFC5322.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    ///The organizer's Profile ID, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    ///The organizer's name, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    ///Whether the organizer corresponds to the calendar on which this copy of the event appears. Read-only. The default is False.
    #[serde(rename = "self")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_: Option<bool>,
}
///The creator of the event. Read-only.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCreator {
    ///Whether the creator corresponds to the calendar on which this copy of the event appears. Read-only. The default is False.
    #[serde(rename = "self")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_: Option<bool>,
    ///The creator's name, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    ///The creator's Profile ID, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    ///The creator's email address, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}
///A gadget that extends this event. Gadgets are deprecated; this structure is instead only used for returning birthday calendar metadata.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventGadget {
    /**The gadget's display mode. Deprecated. Possible values are:
- "icon" - The gadget displays next to the event's title in the calendar view.
- "chip" - The gadget displays when the event is clicked.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    ///The gadget's type. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    ///The gadget's icon URL. The URL scheme must be HTTPS. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_link: Option<String>,
    ///The gadget's width in pixels. The width must be an integer greater than 0. Optional. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    ///Preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferences: Option<std::collections::HashMap<String, String>>,
    ///The gadget's height in pixels. The height must be an integer greater than 0. Optional. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    ///The gadget's title. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    ///The gadget's URL. The URL scheme must be HTTPS. Deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}
///Source from which the event was created. For example, a web page, an email message or any document identifiable by an URL with HTTP or HTTPS scheme. Can only be seen or modified by the creator of the event.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSource {
    ///Title of the source; for example a title of a web page or an email subject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    ///URL of the source pointing to a resource. The URL scheme must be HTTP or HTTPS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    ///An absolute link to the Google Hangout associated with this event. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hangout_link: Option<String>,
    ///Whether attendees other than the organizer can see who the event's attendees are. Optional. The default is True.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guests_can_see_other_guests: Option<bool>,
    ///Information about the event's reminders for the authenticated user. Note that changing reminders does not also change the updated property of the enclosing event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminders: Option<EventReminders>,
    ///The (inclusive) start time of the event. For a recurring event, this is the start time of the first instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<EventDateTime>,
    ///Extended properties of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_properties: Option<EventExtendedProperties>,
    /**Visibility of the event. Optional. Possible values are:
- "default" - Uses the default visibility for events on the calendar. This is the default value.
- "public" - The event is public and event details are visible to all readers of the calendar.
- "private" - The event is private and only event attendees may view event details.
- "confidential" - The event is private. This value is provided for compatibility reasons.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /**File attachments for the event.
In order to modify attachments the supportsAttachments request parameter should be set to true.
There can be at most 25 attachments per event,*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<EventAttachment>>,
    ///The organizer of the event. If the organizer is also an attendee, this is indicated with a separate entry in attendees with the organizer field set to True. To change the organizer, use the move operation. Read-only, except when importing an event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizer: Option<EventOrganizer>,
    ///Whether attendees may have been omitted from the event's representation. When retrieving an event, this may be due to a restriction specified by the maxAttendee query parameter. When updating an event, this can be used to only update the participant's response. Optional. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees_omitted: Option<bool>,
    ///An absolute link to this event in the Google Calendar Web UI. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_link: Option<String>,
    ///For an instance of a recurring event, this is the id of the recurring event to which this instance belongs. Immutable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    ///Whether the end time is actually unspecified. An end time is still provided for compatibility reasons, even if this attribute is set to True. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time_unspecified: Option<bool>,
    ///Creation time of the event (as a RFC3339 timestamp). Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    ///Last modification time of the main event data (as a RFC3339 timestamp). Updating event reminders will not cause this to change. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    ///Geographic location of the event as free-form text. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    ///The attendees of the event. See the Events with attendees guide for more information on scheduling events with other calendar users. Service accounts need to use domain-wide delegation of authority to populate the attendee list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<EventAttendee>>,
    ///ETag of the resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    ///The creator of the event. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<EventCreator>,
    ///Whether anyone can invite themselves to the event (deprecated). Optional. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anyone_can_add_self: Option<bool>,
    ///Whether attendees other than the organizer can invite others to the event. Optional. The default is True.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guests_can_invite_others: Option<bool>,
    /**Opaque identifier of the event. When creating new single or recurring events, you can specify their IDs. Provided IDs must follow these rules:
- characters allowed in the ID are those used in base32hex encoding, i.e. lowercase letters a-v and digits 0-9, see section 3.1.2 in RFC2938
- the length of the ID must be between 5 and 1024 characters
- the ID must be unique per calendar  Due to the globally distributed nature of the system, we cannot guarantee that ID collisions will be detected at event creation time. To minimize the risk of collisions we recommend using an established UUID algorithm such as one described in RFC4122.
If you do not specify an ID, it will be automatically generated by the server.
Note that the icalUID and the id are not identical and only one of them should be supplied at event creation time. One difference in their semantics is that in recurring events, all occurrences of one event have different ids while they all share the same icalUIDs.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    ///Description of the event. Can contain HTML. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /**Event unique identifier as defined in RFC5545. It is used to uniquely identify events accross calendaring systems and must be supplied when importing events via the import method.
Note that the iCalUID and the id are not identical and only one of them should be supplied at event creation time. One difference in their semantics is that in recurring events, all occurrences of one event have different ids while they all share the same iCalUIDs. To retrieve an event using its iCalUID, call the events.list method using the iCalUID parameter. To retrieve an event using its id, call the events.get method.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i_cal_uid: Option<String>,
    ///For an instance of a recurring event, this is the time at which this event would start according to the recurrence data in the recurring event identified by recurringEventId. It uniquely identifies the instance within the recurring event series even if the instance was moved to a different time. Immutable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_start_time: Option<EventDateTime>,
    ///Working location event data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_location_properties: Option<EventWorkingLocationProperties>,
    ///Whether attendees other than the organizer can modify the event. Optional. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guests_can_modify: Option<bool>,
    ///The conference-related information, such as details of a Google Meet conference. To create new conference details use the createRequest field. To persist your changes, remember to set the conferenceDataVersion request parameter to 1 for all event modification requests. Warning: Reusing Google Meet conference data across different events can cause access issues and expose meeting details to unintended users. To help ensure meeting privacy, always generate a unique conference for each event by using the createRequest field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_data: Option<ConferenceData>,
    ///Sequence number as per iCalendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i32>,
    ///List of RRULE, EXRULE, RDATE and EXDATE lines for a recurring event, as specified in RFC5545. Note that DTSTART and DTEND lines are not allowed in this field; event start and end times are specified in the start and end fields. This field is omitted for single events or instances of recurring events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Vec<String>>,
    ///Out of office event data. Used if eventType is outOfOffice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_office_properties: Option<EventOutOfOfficeProperties>,
    /**Whether the event blocks time on the calendar. Optional. Possible values are:
- "opaque" - Default value. The event does block time on the calendar. This is equivalent to setting Show me as to Busy in the Calendar UI.
- "transparent" - The event does not block time on the calendar. This is equivalent to setting Show me as to Available in the Calendar UI.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,
    ///A gadget that extends this event. Gadgets are deprecated; this structure is instead only used for returning birthday calendar metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gadget: Option<EventGadget>,
    ///Source from which the event was created. For example, a web page, an email message or any document identifiable by an URL with HTTP or HTTPS scheme. Can only be seen or modified by the creator of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<EventSource>,
    ///Type of the resource ("calendar#event").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /**Status of the event. Optional. Possible values are:
- "confirmed" - The event is confirmed. This is the default status.
- "tentative" - The event is tentatively confirmed.
- "cancelled" - The event is cancelled (deleted). The list method returns cancelled events only on incremental sync (when syncToken or updatedMin are specified) or if the showDeleted flag is set to true. The get method always returns them.
A cancelled status represents two different states depending on the event type:
- Cancelled exceptions of an uncancelled recurring event indicate that this instance should no longer be presented to the user. Clients should store these events for the lifetime of the parent recurring event.
Cancelled exceptions are only guaranteed to have values for the id, recurringEventId and originalStartTime fields populated. The other fields might be empty.
- All other cancelled events represent deleted events. Clients should remove their locally synced copies. Such cancelled events will eventually disappear, so do not rely on them being available indefinitely.
Deleted events are only guaranteed to have the id field populated.   On the organizer's calendar, cancelled events continue to expose event details (summary, location, etc.) so that they can be restored (undeleted). Similarly, the events to which the user was invited and that they manually removed continue to provide details. However, incremental sync requests with showDeleted set to false will not return these details.
If an event changes its organizer (for example via the move operation) and the original organizer is not on the attendee list, it will leave behind a cancelled event where only the id field is guaranteed to be populated.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    ///Birthday or special event data. Used if eventType is "birthday". Immutable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday_properties: Option<EventBirthdayProperties>,
    ///Focus Time event data. Used if eventType is focusTime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_time_properties: Option<EventFocusTimeProperties>,
    ///The color of the event. This is an ID referring to an entry in the event section of the colors definition (see the  colors endpoint). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    /**Specific type of the event. This cannot be modified after the event is created. Possible values are:
- "birthday" - A special all-day event with an annual recurrence.
- "default" - A regular event or not further specified.
- "focusTime" - A focus-time event.
- "fromGmail" - An event from Gmail. This type of event cannot be created.
- "outOfOffice" - An out-of-office event.
- "workingLocation" - A working location event.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    ///Title of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    ///If set to True, Event propagation is disabled. Note that it is not the same thing as Private event properties. Optional. Immutable. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_copy: Option<bool>,
    ///Whether this is a locked event copy where no changes can be made to the main event fields "summary", "description", "location", "start", "end" or "recurrence". The default is False. Read-Only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    ///The (exclusive) end time of the event. For a recurring event, this is the end time of the first instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<EventDateTime>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceParameters {
    ///Additional add-on specific data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_on_parameters: Option<ConferenceParametersAddOnParameters>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    ///The time zone in which the time is specified. (Formatted as an IANA Time Zone Database name, e.g. "Europe/Zurich".) For recurring events this field is required and specifies the time zone in which the recurrence is expanded. For single events this field is optional and indicates a custom time zone for the event start/end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    ///The date, in the format "yyyy-mm-dd", if this is an all-day event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    ///The time, as a combined date-time value (formatted according to RFC3339). A time zone offset is required unless a time zone is explicitly specified in timeZone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFocusTimeProperties {
    ///The status to mark the user in Chat and related products. This can be available or doNotDisturb.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_status: Option<String>,
    ///Response message to set if an existing event or new invitation is automatically declined by Calendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decline_message: Option<String>,
    ///Whether to decline meeting invitations which overlap Focus Time events. Valid values are declineNone, meaning that no meeting invitations are declined; declineAllConflictingInvitations, meaning that all conflicting meeting invitations that conflict with the event are declined; and declineOnlyNewConflictingInvitations, meaning that only new conflicting meeting invitations which arrive while the Focus Time event is present are to be declined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_decline_mode: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventBirthdayProperties {
    ///Custom type label specified for this event. This is populated if birthdayProperties.type is set to "custom". Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_type_name: Option<String>,
    /**Type of birthday or special event. Possible values are:
- "anniversary" - An anniversary other than birthday. Always has a contact.
- "birthday" - A birthday event. This is the default value.
- "custom" - A special date whose label is further specified in the customTypeName field. Always has a contact.
- "other" - A special date which does not fall into the other categories, and does not have a custom label. Always has a contact.
- "self" - Calendar owner's own birthday. Cannot have a contact.  The Calendar API only supports creating events with the type "birthday". The type cannot be changed after the event is created.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    ///Resource name of the contact this birthday event is linked to. This can be used to fetch contact details from People API. Format: "people/c12345". Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceSolution {
    ///The key which can uniquely identify the conference solution for this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<ConferenceSolutionKey>,
    ///The user-visible name of this solution. Not localized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    ///The user-visible icon for this solution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_uri: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceData {
    /**A request to generate a new conference and attach it to the event. The data is generated asynchronously. To see whether the data is present check the status field.
Either conferenceSolution and at least one entryPoint, or createRequest is required.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_request: Option<CreateConferenceRequest>,
    /**The ID of the conference.
Can be used by developers to keep track of conferences, should not be displayed to users.
The ID value is formed differently for each conference solution type:
- eventHangout: ID is not set. (This conference type is deprecated.)
- eventNamedHangout: ID is the name of the Hangout. (This conference type is deprecated.)
- hangoutsMeet: ID is the 10-letter meeting code, for example aaa-bbbb-ccc.
- addOn: ID is defined by the third-party provider.  Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_id: Option<String>,
    /**Information about individual conference entry points, such as URLs or phone numbers.
All of them must belong to the same conference.
Either conferenceSolution and at least one entryPoint, or createRequest is required.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_points: Option<Vec<EntryPoint>>,
    ///Additional notes (such as instructions from the domain administrator, legal notices) to display to the user. Can contain HTML. The maximum length is 2048 characters. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /**The signature of the conference data.
Generated on server side.
Unset for a conference with a failed create request.
Optional for a conference with a pending create request.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    ///Additional properties related to a conference. An example would be a solution-specific setting for enabling video streaming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<ConferenceParameters>,
    /**The conference solution, such as Google Meet.
Unset for a conference with a failed create request.
Either conferenceSolution and at least one entryPoint, or createRequest is required.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_solution: Option<ConferenceSolution>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttachment {
    ///Internet media type (MIME type) of the attachment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /**ID of the attached file. Read-only.
For Google Drive files, this is the ID of the corresponding Files resource entry in the Drive API.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    ///URL link to the attachment's icon. This field can only be modified for custom third-party attachments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_link: Option<String>,
    ///Attachment title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /**URL link to the attachment.
For adding Google Drive file attachments use the same format as in alternateLink property of the Files resource in the Drive API.
Required when adding an attachment.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_url: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceSolutionKey {
    /**The conference solution type.
If a client encounters an unfamiliar or empty type, it should still be able to display the entry points. However, it should disallow modifications.
The possible values are:
- "eventHangout" for Hangouts for consumers (deprecated; existing events may show this conference solution type but new conferences cannot be created)
- "eventNamedHangout" for classic Hangouts for Google Workspace users (deprecated; existing events may show this conference solution type but new conferences cannot be created)
- "hangoutsMeet" for Google Meet (http://meet.google.com)
- "addOn" for 3P conference providers*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConferenceRequest {
    /**The client-generated unique ID for this request.
Clients should regenerate this ID for every new request. If an ID provided is the same as for the previous request, the request is ignored.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    ///The conference solution, such as Hangouts or Google Meet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_solution_key: Option<ConferenceSolutionKey>,
    ///The status of the conference create request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ConferenceRequestStatus>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceRequestStatus {
    /**The current status of the conference create request. Read-only.
The possible values are:
- "pending": the conference create request is still being processed.
- "success": the conference create request succeeded, the entry points are populated.
- "failure": the conference create request failed, there are no entry points.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    /**The CLDR/ISO 3166 region code for the country associated with this phone access. Example: "SE" for Sweden.
Calendar backend will populate this field only for EntryPointType.PHONE.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_code: Option<String>,
    /**The meeting code to access the conference. The maximum length is 128 characters.
When creating new conference data, populate only the subset of {meetingCode, accessCode, passcode, password, pin} fields that match the terminology that the conference provider uses. Only the populated fields should be displayed.
Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_code: Option<String>,
    /**The password to access the conference. The maximum length is 128 characters.
When creating new conference data, populate only the subset of {meetingCode, accessCode, passcode, password, pin} fields that match the terminology that the conference provider uses. Only the populated fields should be displayed.
Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /**The passcode to access the conference. The maximum length is 128 characters.
When creating new conference data, populate only the subset of {meetingCode, accessCode, passcode, password, pin} fields that match the terminology that the conference provider uses. Only the populated fields should be displayed.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passcode: Option<String>,
    /**The type of the conference entry point.
Possible values are:
- "video" - joining a conference over HTTP. A conference can have zero or one video entry point.
- "phone" - joining a conference by dialing a phone number. A conference can have zero or more phone entry points.
- "sip" - joining a conference over SIP. A conference can have zero or one sip entry point.
- "more" - further conference joining instructions, for example additional phone numbers. A conference can have zero or one more entry point. A conference with only a more entry point is not a valid conference.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point_type: Option<String>,
    /**The label for the URI. Visible to end users. Not localized. The maximum length is 512 characters.
Examples:
- for video: meet.google.com/aaa-bbbb-ccc
- for phone: +1 123 268 2601
- for sip: 12345678@altostrat.com
- for more: should not be filled
Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    ///Features of the entry point, such as being toll or toll-free. One entry point can have multiple features. However, toll and toll-free cannot be both set on the same entry point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point_features: Option<Vec<String>>,
    /**The access code to access the conference. The maximum length is 128 characters.
When creating new conference data, populate only the subset of {meetingCode, accessCode, passcode, password, pin} fields that match the terminology that the conference provider uses. Only the populated fields should be displayed.
Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_code: Option<String>,
    /**The URI of the entry point. The maximum length is 1300 characters.
Format:
- for video, http: or https: schema is required.
- for phone, tel: schema is required. The URI should include the entire dial sequence (e.g., tel:+12345678900,,,123456789;1234).
- for sip, sip: schema is required, e.g., sip:12345678@myprovider.com.
- for more, http: or https: schema is required.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /**The PIN to access the conference. The maximum length is 128 characters.
When creating new conference data, populate only the subset of {meetingCode, accessCode, passcode, password, pin} fields that match the terminology that the conference provider uses. Only the populated fields should be displayed.
Optional.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttendee {
    ///The attendee's response comment. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    ///Whether the attendee is the organizer of the event. Read-only. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizer: Option<bool>,
    ///Whether this entry represents the calendar on which this copy of the event appears. Read-only. The default is False.
    #[serde(rename = "self")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_: Option<bool>,
    ///The attendee's name, if available. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    ///The attendee's Profile ID, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    ///Whether the attendee is a resource. Can only be set when the attendee is added to the event for the first time. Subsequent modifications are ignored. Optional. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<bool>,
    ///Number of additional guests. Optional. The default is 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_guests: Option<i32>,
    /**The attendee's email address, if available. This field must be present when adding an attendee. It must be a valid email address as per RFC5322.
Required when adding an attendee.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /**The attendee's response status. Possible values are:
- "needsAction" - The attendee has not responded to the invitation (recommended for new events).
- "declined" - The attendee has declined the invitation.
- "tentative" - The attendee has tentatively accepted the invitation.
- "accepted" - The attendee has accepted the invitation.  Warning: If you add an event using the values declined, tentative, or accepted, attendees with the "Add invitations to my calendar" setting set to "When I respond to invitation in email" or "Only if the sender is known" might have their response reset to needsAction and won't see an event in their calendar unless they change their response in the event invitation email. Furthermore, if more than 200 guests are invited to the event, response status is not propagated to the guests.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
    ///Whether this is an optional attendee. Optional. The default is False.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
}
///If present, specifies that the user is working from a custom location.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventWorkingLocationPropertiesCustomLocation {
    ///An optional extra label for additional information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
///If present, specifies that the user is working from an office.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventWorkingLocationPropertiesOfficeLocation {
    ///An optional floor section identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_section_id: Option<String>,
    ///The office name that's displayed in Calendar Web and Mobile clients. We recommend you reference a building name in the organization's Resources database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    ///An optional building identifier. This should reference a building ID in the organization's Resources database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub building_id: Option<String>,
    ///An optional desk identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desk_id: Option<String>,
    ///An optional floor identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_id: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventWorkingLocationProperties {
    ///If present, specifies that the user is working from a custom location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_location: Option<EventWorkingLocationPropertiesCustomLocation>,
    ///If present, specifies that the user is working from an office.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub office_location: Option<EventWorkingLocationPropertiesOfficeLocation>,
    ///If present, specifies that the user is working at home.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_office: Option<serde_json::Value>,
    /**Type of the working location. Possible values are:
- "homeOffice" - The user is working at home.
- "officeLocation" - The user is working from an office.
- "customLocation" - The user is working from a custom location.  Any details are specified in a sub-field of the specified name, but this field may be missing if empty. Any other fields are ignored.
Required when adding working location properties.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceParametersAddOnParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<std::collections::HashMap<String, String>>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventOutOfOfficeProperties {
    ///Response message to set if an existing event or new invitation is automatically declined by Calendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decline_message: Option<String>,
    ///Whether to decline meeting invitations which overlap Out of office events. Valid values are declineNone, meaning that no meeting invitations are declined; declineAllConflictingInvitations, meaning that all conflicting meeting invitations that conflict with the event are declined; and declineOnlyNewConflictingInvitations, meaning that only new conflicting meeting invitations which arrive while the Out of office event is present are to be declined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_decline_mode: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Events {
    ///ETag of the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    ///Token used at a later point in time to retrieve only the entries that have changed since this result was returned. Omitted if further results are available, in which case nextPageToken is provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_sync_token: Option<String>,
    ///The default reminders on the calendar for the authenticated user. These reminders apply to all events on this calendar that do not explicitly override them (i.e. do not have reminders.useDefault set to True).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reminders: Option<Vec<EventReminder>>,
    ///Title of the calendar. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    ///The time zone of the calendar. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    ///Type of the collection ("calendar#events").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    ///Description of the calendar. Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    ///List of events on the calendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<Event>>,
    ///Last modification time of the calendar (as a RFC3339 timestamp). Read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /**The user's access role for this calendar. Read-only. Possible values are:
- "none" - The user has no access.
- "freeBusyReader" - The user has read access to free/busy information.
- "reader" - The user has read access to the calendar. Private events will appear to users with reader access, but event details will be hidden.
- "writer" - The user has read and write access to the calendar. Private events will appear to users with writer access, and event details will be visible.
- "owner" - The user has manager access to the calendar. This role has all of the permissions of the writer role with the additional ability to see and modify access levels of other users.
Important: the owner role is different from the calendar's data owner. A calendar has a single data owner, but can have multiple users with owner role.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_role: Option<String>,
    ///Token used to access the next page of this result. Omitted if no further results are available, in which case nextSyncToken is provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReminder {
    /**Number of minutes before the start of the event when the reminder should trigger. Valid values are between 0 and 40320 (4 weeks in minutes).
Required when adding a reminder.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i32>,
    /**The method used by this reminder. Possible values are:
- "email" - Reminders are sent via email.
- "popup" - Reminders are sent via a UI popup.
Required when adding a reminder.*/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}
///Query/path parameters for `calendar.events.delete`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_updates: Option<String>,
}
///Query/path parameters for `calendar.events.insert`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_attachments: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_data_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_updates: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<i32>,
}
///Query/path parameters for `calendar.events.patch`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_include_email: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_updates: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_data_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_attachments: Option<bool>,
}
///Query/path parameters for `calendar.events.list`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hidden_invitations: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_extended_property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_events: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_extended_property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_types: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_deleted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i_cal_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_include_email: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
}
///Query/path parameters for `calendar.events.get`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_include_email: Option<bool>,
}
pub static DELETE_ACTION: ActionDescriptor = ActionDescriptor {
    id: "calendar.events.delete",
    service: "calendar",
    resource_path: "events",
    method_name: "delete",
    http_method: "DELETE",
    description: "Deletes an event.",
    path_template: "calendars/{calendarId}/events/{eventId}",
    base_url: "https://www.googleapis.com/calendar/v3/",
    scopes: &[
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/calendar.app.created",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.events.owned",
    ],
    parameters: &[
        ParamDescriptor {
            name: "eventId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Event identifier.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "calendarId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Calendar identifier. To retrieve calendar IDs call the calendarList.list method. If you want to access the primary calendar of the currently logged in user, use the \"primary\" keyword.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendNotifications",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated. Please use sendUpdates instead.\n\nWhether to send notifications about the deletion of the event. Note that some emails might still be sent even if you set the value to false. The default is false.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendUpdates",
            param_type: "string",
            location: "query",
            required: false,
            description: "Guests who should receive notifications about the deletion of the event.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: None,
    response_body_schema: None,
    supports_pagination: false,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static INSERT_ACTION: ActionDescriptor = ActionDescriptor {
    id: "calendar.events.insert",
    service: "calendar",
    resource_path: "events",
    method_name: "insert",
    http_method: "POST",
    description: "Creates an event.",
    path_template: "calendars/{calendarId}/events",
    base_url: "https://www.googleapis.com/calendar/v3/",
    scopes: &[
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/calendar.app.created",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.events.owned",
    ],
    parameters: &[
        ParamDescriptor {
            name: "calendarId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Calendar identifier. To retrieve calendar IDs call the calendarList.list method. If you want to access the primary calendar of the currently logged in user, use the \"primary\" keyword.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "supportsAttachments",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Whether API client performing operation supports event attachments. Optional. The default is False.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "conferenceDataVersion",
            param_type: "integer",
            location: "query",
            required: false,
            description: "Version number of conference data supported by the API client. Version 0 assumes no conference data support and ignores conference data in the event's body. Version 1 enables support for copying of ConferenceData as well as for creating new conferences using the createRequest field of conferenceData. The default is 0.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendNotifications",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated. Please use sendUpdates instead.\n\nWhether to send notifications about the creation of the new event. Note that some emails might still be sent even if you set the value to false. The default is false.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendUpdates",
            param_type: "string",
            location: "query",
            required: false,
            description: "Whether to send notifications about the creation of the new event. Note that some emails might still be sent. The default is false.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxAttendees",
            param_type: "integer",
            location: "query",
            required: false,
            description: "The maximum number of attendees to include in the response. If there are more than the specified number of attendees, only the participant is returned. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: Some("Event"),
    response_body_schema: Some("Event"),
    supports_pagination: false,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static PATCH_ACTION: ActionDescriptor = ActionDescriptor {
    id: "calendar.events.patch",
    service: "calendar",
    resource_path: "events",
    method_name: "patch",
    http_method: "PATCH",
    description: "Updates an event. This method supports patch semantics.",
    path_template: "calendars/{calendarId}/events/{eventId}",
    base_url: "https://www.googleapis.com/calendar/v3/",
    scopes: &[
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/calendar.app.created",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.events.owned",
    ],
    parameters: &[
        ParamDescriptor {
            name: "eventId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Event identifier.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "calendarId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Calendar identifier. To retrieve calendar IDs call the calendarList.list method. If you want to access the primary calendar of the currently logged in user, use the \"primary\" keyword.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "alwaysIncludeEmail",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated and ignored. A value will always be returned in the email field for the organizer, creator and attendees, even if no real email address is available (i.e. a generated, non-working value will be provided).",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendNotifications",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated. Please use sendUpdates instead.\n\nWhether to send notifications about the event update (for example, description changes, etc.). Note that some emails might still be sent even if you set the value to false. The default is false.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sendUpdates",
            param_type: "string",
            location: "query",
            required: false,
            description: "Guests who should receive notifications about the event update (for example, title changes, etc.).",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxAttendees",
            param_type: "integer",
            location: "query",
            required: false,
            description: "The maximum number of attendees to include in the response. If there are more than the specified number of attendees, only the participant is returned. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "conferenceDataVersion",
            param_type: "integer",
            location: "query",
            required: false,
            description: "Version number of conference data supported by the API client. Version 0 assumes no conference data support and ignores conference data in the event's body. Version 1 enables support for copying of ConferenceData as well as for creating new conferences using the createRequest field of conferenceData. The default is 0.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "supportsAttachments",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Whether API client performing operation supports event attachments. Optional. The default is False.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: Some("Event"),
    response_body_schema: Some("Event"),
    supports_pagination: false,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static LIST_ACTION: ActionDescriptor = ActionDescriptor {
    id: "calendar.events.list",
    service: "calendar",
    resource_path: "events",
    method_name: "list",
    http_method: "GET",
    description: "Returns events on the specified calendar.",
    path_template: "calendars/{calendarId}/events",
    base_url: "https://www.googleapis.com/calendar/v3/",
    scopes: &[
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/calendar.app.created",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.events.freebusy",
        "https://www.googleapis.com/auth/calendar.events.owned",
        "https://www.googleapis.com/auth/calendar.events.owned.readonly",
        "https://www.googleapis.com/auth/calendar.events.public.readonly",
        "https://www.googleapis.com/auth/calendar.events.readonly",
        "https://www.googleapis.com/auth/calendar.readonly",
    ],
    parameters: &[
        ParamDescriptor {
            name: "calendarId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Calendar identifier. To retrieve calendar IDs call the calendarList.list method. If you want to access the primary calendar of the currently logged in user, use the \"primary\" keyword.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "showHiddenInvitations",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Whether to include hidden invitations in the result. Optional. The default is False.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "privateExtendedProperty",
            param_type: "string",
            location: "query",
            required: false,
            description: "Extended properties constraint specified as propertyName=value. Matches only private properties. This parameter might be repeated multiple times to return events that match all given constraints.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "updatedMin",
            param_type: "string",
            location: "query",
            required: false,
            description: "Lower bound for an event's last modification time (as a RFC3339 timestamp) to filter by. When specified, entries deleted since this time will always be included regardless of showDeleted. Optional. The default is not to filter by last modification time.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "timeMax",
            param_type: "string",
            location: "query",
            required: false,
            description: "Upper bound (exclusive) for an event's start time to filter by. Optional. The default is not to filter by start time. Must be an RFC3339 timestamp with mandatory time zone offset, for example, 2011-06-03T10:00:00-07:00, 2011-06-03T10:00:00Z. Milliseconds may be provided but are ignored. If timeMin is set, timeMax must be greater than timeMin.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "timeZone",
            param_type: "string",
            location: "query",
            required: false,
            description: "Time zone used in the response. Optional. The default is the time zone of the calendar.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "pageToken",
            param_type: "string",
            location: "query",
            required: false,
            description: "Token specifying which result page to return. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "singleEvents",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Whether to expand recurring events into instances and only return single one-off events and instances of recurring events, but not the underlying recurring events themselves. Optional. The default is False.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "syncToken",
            param_type: "string",
            location: "query",
            required: false,
            description: "Token obtained from the nextSyncToken field returned on the last page of results from the previous list request. It makes the result of this list request contain only entries that have changed since then. All events deleted since the previous list request will always be in the result set and it is not allowed to set showDeleted to False.\nThere are several query parameters that cannot be specified together with nextSyncToken to ensure consistency of the client state.\n\nThese are: \n- iCalUID \n- orderBy \n- privateExtendedProperty \n- q \n- sharedExtendedProperty \n- timeMin \n- timeMax \n- updatedMin All other query parameters should be the same as for the initial synchronization to avoid undefined behavior. If the syncToken expires, the server will respond with a 410 GONE response code and the client should clear its storage and perform a full synchronization without any syncToken.\nLearn more about incremental synchronization.\nOptional. The default is to return all entries.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "timeMin",
            param_type: "string",
            location: "query",
            required: false,
            description: "Lower bound (exclusive) for an event's end time to filter by. Optional. The default is not to filter by end time. Must be an RFC3339 timestamp with mandatory time zone offset, for example, 2011-06-03T10:00:00-07:00, 2011-06-03T10:00:00Z. Milliseconds may be provided but are ignored. If timeMax is set, timeMin must be smaller than timeMax.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "orderBy",
            param_type: "string",
            location: "query",
            required: false,
            description: "The order of the events returned in the result. Optional. The default is an unspecified, stable order.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "sharedExtendedProperty",
            param_type: "string",
            location: "query",
            required: false,
            description: "Extended properties constraint specified as propertyName=value. Matches only shared properties. This parameter might be repeated multiple times to return events that match all given constraints.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "eventTypes",
            param_type: "string",
            location: "query",
            required: false,
            description: "Event types to return. Optional. This parameter can be repeated multiple times to return events of different types. If unset, returns all event types.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "showDeleted",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Whether to include deleted events (with status equals \"cancelled\") in the result. Cancelled instances of recurring events (but not the underlying recurring event) will still be included if showDeleted and singleEvents are both False. If showDeleted and singleEvents are both True, only single instances of deleted events (but not the underlying recurring events) are returned. Optional. The default is False.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "iCalUID",
            param_type: "string",
            location: "query",
            required: false,
            description: "Specifies an event ID in the iCalendar format to be provided in the response. Optional. Use this if you want to search for an event by its iCalendar ID.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "q",
            param_type: "string",
            location: "query",
            required: false,
            description: "Free text search terms to find events that match these terms in the following fields:\n\n- summary \n- description \n- location \n- attendee's displayName \n- attendee's email \n- organizer's displayName \n- organizer's email \n- workingLocationProperties.officeLocation.buildingId \n- workingLocationProperties.officeLocation.deskId \n- workingLocationProperties.officeLocation.label \n- workingLocationProperties.customLocation.label \nThese search terms also match predefined keywords against all display title translations of working location, out-of-office, and focus-time events. For example, searching for \"Office\" or \"Bureau\" returns working location events of type officeLocation, whereas searching for \"Out of office\" or \"Abwesend\" returns out-of-office events. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxAttendees",
            param_type: "integer",
            location: "query",
            required: false,
            description: "The maximum number of attendees to include in the response. If there are more than the specified number of attendees, only the participant is returned. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "alwaysIncludeEmail",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated and ignored.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxResults",
            param_type: "integer",
            location: "query",
            required: false,
            description: "Maximum number of events returned on one result page. The number of events in the resulting page may be less than this value, or none at all, even if there are more events matching the query. Incomplete pages can be detected by a non-empty nextPageToken field in the response. By default the value is 250 events. The page size can never be larger than 2500 events. Optional.",
            default_value: Some("\"250\""),
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: None,
    response_body_schema: Some("Events"),
    supports_pagination: true,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static GET_ACTION: ActionDescriptor = ActionDescriptor {
    id: "calendar.events.get",
    service: "calendar",
    resource_path: "events",
    method_name: "get",
    http_method: "GET",
    description: "Returns an event based on its Google Calendar ID. To retrieve an event using its iCalendar ID, call the events.list method using the iCalUID parameter.",
    path_template: "calendars/{calendarId}/events/{eventId}",
    base_url: "https://www.googleapis.com/calendar/v3/",
    scopes: &[
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/calendar.app.created",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.events.freebusy",
        "https://www.googleapis.com/auth/calendar.events.owned",
        "https://www.googleapis.com/auth/calendar.events.owned.readonly",
        "https://www.googleapis.com/auth/calendar.events.public.readonly",
        "https://www.googleapis.com/auth/calendar.events.readonly",
        "https://www.googleapis.com/auth/calendar.readonly",
    ],
    parameters: &[
        ParamDescriptor {
            name: "calendarId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Calendar identifier. To retrieve calendar IDs call the calendarList.list method. If you want to access the primary calendar of the currently logged in user, use the \"primary\" keyword.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "eventId",
            param_type: "string",
            location: "path",
            required: true,
            description: "Event identifier.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "maxAttendees",
            param_type: "integer",
            location: "query",
            required: false,
            description: "The maximum number of attendees to include in the response. If there are more than the specified number of attendees, only the participant is returned. Optional.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "timeZone",
            param_type: "string",
            location: "query",
            required: false,
            description: "Time zone used in the response. Optional. The default is the time zone of the calendar.",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
        ParamDescriptor {
            name: "alwaysIncludeEmail",
            param_type: "boolean",
            location: "query",
            required: false,
            description: "Deprecated and ignored. A value will always be returned in the email field for the organizer, creator and attendees, even if no real email address is available (i.e. a generated, non-working value will be provided).",
            default_value: None,
            enum_values: None,
            deprecated: false,
        },
    ],
    request_body_schema: None,
    response_body_schema: Some("Event"),
    supports_pagination: false,
    supports_media_upload: false,
    supports_media_download: false,
    deprecated: false,
};
pub static ALL_ACTIONS: &[&ActionDescriptor] = &[
    &DELETE_ACTION,
    &INSERT_ACTION,
    &PATCH_ACTION,
    &LIST_ACTION,
    &GET_ACTION,
];
