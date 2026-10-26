package dev.omnibuild.module

/**
 * The E2E tests' native module, as a module's author writes one: a class in
 * a library of its own, which the fixture shell finds by its name
 * (`Class.forName`), as the runtime it names, `omnibuild-e2e@1`, registers
 * a module. Nothing compiled the shell against it, and its DEX file has its
 * own classes alone: the Kotlin it calls is the shell's.
 */
class OmniBuildModule {
    /**
     * What the module says when the shell asks, with the name of its class,
     * which only its own DEX file defines.
     */
    fun report(): String = listOf("OmniBuildModule", "in", javaClass.name).joinToString(" ")
}
