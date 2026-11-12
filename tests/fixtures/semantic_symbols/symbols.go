package go_sample
type Runner interface { Run(value int) }
type Engine struct { Value int }
func (e *Engine) Run(value int) {}
func Sum(a, b int, rest ...int) int { return a+b }
