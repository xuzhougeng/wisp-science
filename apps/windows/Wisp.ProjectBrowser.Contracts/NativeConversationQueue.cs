using System.Buffers.Binary;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeQueueItem(string Id, string Digest, string State, string Message, string[] Attachments, NativeComposerReference[] References)
{
    [JsonIgnore] public string EditText
    {
        get
        {
            if (Attachments.Length == 0) return Message;
            var block = "Uploaded files: " + string.Join(", ", Attachments);
            return Message == block ? "" : Message.EndsWith("\n\n" + block, StringComparison.Ordinal) ? Message[..^(block.Length + 2)] : Message;
        }
    }
}
public sealed record NativeQueueOutcome(string Id, string State);
public sealed record NativeQueueSnapshot(NativeQueueItem[] Items, NativeQueueOutcome[] Outcomes, bool CanCutIn)
{
    [JsonIgnore] public bool Valid => Items != null && Outcomes != null && Items.Select(item => item.Id).Distinct().Count() == Items.Length
        && Items.All(item => ValidId(item.Id) && !string.IsNullOrEmpty(item.Digest) && item.State is "queued" or "cutin_pending"
            && item.Message != null && item.Attachments != null && item.References != null && item.References.All(reference => reference.Valid))
        && Outcomes.All(outcome => ValidId(outcome.Id) && outcome.State is "started" or "completed" or "failed" or "cancelled" or "superseded");
    public static bool ValidId(string id) => ulong.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out var number)
        && number.ToString(CultureInfo.InvariantCulture) == id;
    public static string RequestQueueId(Guid request) => BinaryPrimitives.ReadUInt64LittleEndian(Convert.FromHexString(request.ToString("N"))).ToString(CultureInfo.InvariantCulture);
}
public sealed record NativeQueueTarget(string ProjectId, string SessionId, NativeQueueItem Item);

public sealed class NativeConversationQueueClient(INativeSettingsClient transport)
{
    public async Task ActAsync(NativeQueueTarget target, JsonObject action, CancellationToken token)
    {
        var result = await transport.InvokeAsync("native_conversation_queue_action", new() {
            ["session_id"] = target.SessionId, ["id"] = target.Item.Id, ["digest"] = target.Item.Digest, ["action"] = action.DeepClone()
        }, target.ProjectId, token).ConfigureAwait(false);
        if (result?["session_id"]?.GetValue<string>() != target.SessionId || result?["id"]?.GetValue<string>() != target.Item.Id)
            throw new InvalidDataException("Queued action response identity mismatch");
    }
}
