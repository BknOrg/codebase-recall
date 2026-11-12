class Worker:
    count: int = 0
    @staticmethod
    def create(value: int = 1):
        return value
    @property
    def value(self):
        return self.count
    @custom_decorator
    def dynamic(self):
        return 1
