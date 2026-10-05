namespace Wisp.ProjectBrowser;

public static class NativeScientificPreview
{
    public static string? Kind(string? path) => Extension(path) switch
    {
        ".pdb" or ".cif" or ".mol2" => "structure",
        ".sdf" or ".mol" or ".smi" or ".smiles" => "molecule",
        ".aln" or ".clustal" or ".clustalw" or ".sto" or ".stockholm" or ".stk" or ".afa" or ".mfa" => "msa",
        ".fasta" or ".fa" or ".fas" or ".fna" or ".faa" or ".ffn" or ".frn" => "fasta",
        _ => null,
    };
    public static string? Format(string? path) => Extension(path) switch
    {
        ".pdb" => "pdb", ".cif" => "cif", ".mol2" => "mol2",
        ".aln" or ".clustal" or ".clustalw" => "clustal",
        ".sto" or ".stockholm" or ".stk" => "stockholm",
        ".afa" or ".mfa" => "fasta",
        _ => null,
    };
    private static string Extension(string? path) => Path.GetExtension(path ?? "").ToLowerInvariant();
}
