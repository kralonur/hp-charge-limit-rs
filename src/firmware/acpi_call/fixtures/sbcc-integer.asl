// Reduced from HP OmniBook 3 14-ha0808no, board 8E2D, BIOS F.06.
// Preserve the argument checks and dispatcher; unrelated modes are omitted.
DefinitionBlock ("", "SSDT", 2, "HPQOEM", "INTTEST", 1)
{
    Scope (\_SB)
    {
        Device (WMID)
        {
            Name (_HID, "PNP0C14")
            Name (BATD, Zero)
            Name (BATC, Zero)
            Method (GBCC, 0, Serialized)
            {
                Return (Package () { Zero, 0x04, Buffer (0x04) { 0, 0, 0, 0 } })
            }
            Method (SBCC, 1, Serialized)
            {
                Sleep (0x03E8)
                If (((Arg0 & 0xFF) == Zero))
                {
                    Switch ((Arg0 & 0xFF00))
                    {
                        Case (Zero) { BATD = Zero BATC = 0x3A }
                        Case (0x0300) { BATD = 0x03 BATC = 0x3A }
                        Case (0x0500) { BATD = 0x05 BATC = 0x3A }
                    }
                }
                Return (Package () { Zero, Zero })
            }
        }
    }
}
