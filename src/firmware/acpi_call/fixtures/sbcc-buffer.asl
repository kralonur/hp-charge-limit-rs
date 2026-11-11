// Reduced from the Victus WMITable: SBCC extracts its mode at offset 0x10.
DefinitionBlock ("", "SSDT", 2, "HPQOEM", "BUFTEST", 1)
{
    Scope (\_SB)
    {
        Device (WMID)
        {
            Name (_HID, "PNP0C14")
            Name (SBTC, Zero)
            Name (SBIS, Zero)
            Method (GBCC, 0, Serialized)
            {
                Return (Package () { Zero, 0x04, Buffer (0x04) { 0, 0, 0, 0 } })
            }
            Method (SBCC, 1, Serialized)
            {
                CreateDWordField (Arg0, 0x10, DBUF)
                Switch ((DBUF & 0x0F01))
                {
                    Case (Zero) { SBTC = Zero SBIS = Zero }
                    Case (0x0300) { SBTC = One }
                    Case (0x0500) { SBIS = One }
                }
                Return (Package () { Zero, Zero })
            }
        }
    }
}
