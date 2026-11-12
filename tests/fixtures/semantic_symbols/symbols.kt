package kotlin_sample
object Registry { val count: Int = 1
    fun run(x: Int = 1) = x
}
fun String.decorate(vararg parts: String): String = this
