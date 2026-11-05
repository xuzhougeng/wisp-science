using System.Runtime.CompilerServices;

// The suites use independent fixtures. Report each failure from one CI run
// instead of hiding the remaining results behind the first exception.
internal sealed class ContractSuiteResults
{
    private readonly List<Exception> failures = [];

    public void Run(Action suite, [CallerArgumentExpression(nameof(suite))] string name = "")
    {
        try { suite(); }
        catch (Exception error) { Record(name, error); }
    }

    public async Task RunAsync(Func<Task> suite, [CallerArgumentExpression(nameof(suite))] string name = "")
    {
        try { await suite(); }
        catch (Exception error) { Record(name, error); }
    }

    private void Record(string name, Exception error)
    {
        Console.Error.WriteLine($"FAILED {name}: {error}");
        failures.Add(new InvalidOperationException(name, error));
    }

    public void ThrowIfFailed()
    {
        if (failures.Count > 0) throw new AggregateException("Windows contract suites failed", failures);
    }
}
